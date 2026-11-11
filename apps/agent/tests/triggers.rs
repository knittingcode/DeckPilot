//! Workflows started by schedules and lifecycle events, end to end with
//! simulated devices, real IPC and an injected clock.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use deckpilot_agent::{Agent, AgentError, AgentOptions, Clock, InstanceLock};
use deckpilot_core::graph::{
    ErrorPolicy, GRAPH_VERSION, Mapping, Node, NodeId, NodeKind, Position, ValueKind,
    WorkflowGraph, WorkflowInput,
};
use deckpilot_core::{
    ActionParameters, CycleDirection, EventField, ExecutionRecord, ExecutionTrigger,
    LifecycleEvent, OnFailure, Os, Profile, ProfileId, Project, ProjectKey, ProjectScope, Schedule,
    TriggerValue, Workflow, WorkflowId, WorkflowTrigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::models;
use deckpilot_ipc::{AgentClient, AgentPaths, ClientKind, KeyMaterial, Request, Response};
use deckpilot_modules::general;
use deckpilot_modules::projects::ProjectCatalog;
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore};
use tokio::sync::oneshot;

const TIMEOUT: Duration = Duration::from_secs(20);

struct Harness {
    paths: AgentPaths,
    key: DatabaseKey,
    mock: MockBackend,
    profile: Profile,
    stop: Option<oneshot::Sender<()>>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// A profile with SHOP, CMR and CMS and an `Announce` log action, plus the
/// workflows `build` makes from that action.
fn start(clock: Clock, build: impl FnOnce(&Profile) -> Vec<Workflow>) -> Harness {
    let dir = std::env::temp_dir().join(format!("deckpilot-triggers-{}", ProfileId::new()));
    let paths = AgentPaths::in_dir(dir);
    paths.create_dirs().unwrap();
    let key = Vault::new(&paths.database)
        .with_test_cost()
        .initialize(&Password::new("test password"))
        .unwrap();
    let store = SqliteStore::open(&paths.database, &Encryption::Key(key.clone())).unwrap();
    let registry = deckpilot_modules::builtin(Os::current()).action_registry();
    let mut profile = Profile::new("Work");
    for project in ["SHOP", "CMR", "CMS"] {
        profile
            .add_project(Project::new(ProjectKey::new(project).unwrap(), project))
            .unwrap();
    }
    profile.actions.push(
        registry
            .create_instance(
                "Announce",
                general::log_id(),
                &ActionParameters::new().with(
                    "message",
                    deckpilot_core::ParameterValue::Text("{project.key}".into()),
                ),
            )
            .unwrap(),
    );
    profile.workflows = build(&profile);
    store.save_profile(&profile).unwrap();
    store.set_active_profile(Some(profile.id)).unwrap();

    let mock = MockBackend::new();
    // Connected while the agent is locked: not an event.
    mock.plug(&models::stream_deck_mk2(), "SIM-0001");
    let lock = InstanceLock::acquire(&paths.lock_file).unwrap();
    let options = AgentOptions {
        paths: paths.clone(),
        backend: Box::new(mock.clone()),
        mock: Some(mock.clone()),
        rescan_interval: Duration::from_millis(50),
        autostart: None,
        open_desktop: None,
        open_window: None,
        key_store: None,
        auto_lock_check: Duration::from_millis(50),
        cancel_grace: Duration::from_secs(10),
        clock,
        notifier: None,
        screen_lock: None,
    };
    let agent = Agent::start(options, lock).unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let _task: tokio::task::JoinHandle<Result<(), AgentError>> =
        tokio::spawn(agent.run(async move {
            let _ = stopped.await;
        }));
    Harness {
        paths,
        key,
        mock,
        profile,
        stop: Some(stop),
    }
}

impl Harness {
    async fn request(&self, request: Request) -> Response {
        let mut client = AgentClient::connect(&self.paths, ClientKind::Desktop)
            .await
            .unwrap();
        client.request(request).await.unwrap()
    }

    async fn unlock(&self) {
        self.request(Request::Unlock {
            key: KeyMaterial::new(self.key.to_hex()),
        })
        .await;
    }

    fn store(&self) -> SqliteStore {
        SqliteStore::open(&self.paths.database, &Encryption::Key(self.key.clone())).unwrap()
    }

    async fn runs(&self, workflow: WorkflowId) -> Vec<ExecutionRecord> {
        match self
            .request(Request::ListExecutions {
                workflow: Some(workflow),
                limit: None,
            })
            .await
        {
            Response::Executions { executions } => {
                let mut records = Vec::new();
                for summary in executions {
                    if let Response::Execution { execution } = self
                        .request(Request::GetExecution {
                            execution: summary.id,
                        })
                        .await
                    {
                        records.push(*execution);
                    }
                }
                records
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Waits until `workflow` has `count` finished runs.
    async fn wait_for_runs(&self, workflow: WorkflowId, count: usize) -> Vec<ExecutionRecord> {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let runs = self.runs(workflow).await;
            let finished = runs
                .iter()
                .filter(|r| r.summary.state.is_finished())
                .count();
            if finished >= count {
                assert_eq!(runs.len(), count, "{runs:#?}");
                return runs;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{count} runs expected, got {runs:#?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn save(&self, change: impl FnOnce(&mut Profile)) {
        let store = self.store();
        let mut profile = store.load_profile(self.profile.id).unwrap().unwrap();
        change(&mut profile);
        store.save_profile(&profile).unwrap();
        self.request(Request::ReloadConfiguration).await;
    }
}

fn steps(name: &str, profile: &Profile, trigger: WorkflowTrigger) -> Workflow {
    let mut workflow = Workflow::new(name).step(profile.actions[0].id, OnFailure::Stop);
    workflow.project_scope = Some(ProjectScope::Execution);
    workflow.triggers.push(trigger);
    workflow
}

fn node(id: &str, kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(id).unwrap(),
        kind,
        label: String::new(),
        position: Position::default(),
    }
}

/// A graph workflow that logs its `serial` input, which a device trigger
/// fills from the device's serial.
fn device_logger(profile: &Profile) -> Workflow {
    let mut workflow = Workflow::new("Log device");
    workflow.project_scope = Some(ProjectScope::Execution);
    workflow.inputs.push(WorkflowInput {
        required: true,
        ..WorkflowInput::new("serial", ValueKind::Text)
    });
    workflow.graph = Some(WorkflowGraph {
        version: GRAPH_VERSION,
        nodes: vec![
            node("start", NodeKind::Trigger),
            node(
                "log",
                NodeKind::Action {
                    action: profile.actions[0].id,
                    overrides: ActionParameters::new(),
                    mappings: [(
                        "message".to_owned(),
                        Mapping::Template {
                            template: "connected {{ steps.start.output.serial }}".into(),
                        },
                    )]
                    .into(),
                    policy: ErrorPolicy::default(),
                },
            ),
        ],
        edges: vec![deckpilot_core::graph::Edge {
            from: NodeId::new("start").unwrap(),
            port: deckpilot_core::graph::Port::Out,
            to: NodeId::new("log").unwrap(),
        }],
    });
    workflow.triggers.push(
        WorkflowTrigger::lifecycle(LifecycleEvent::DeviceConnected).with_input(
            "serial",
            TriggerValue::Event {
                field: EventField::DeviceId,
            },
        ),
    );
    workflow
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_events_start_workflows_of_the_active_profile() {
    let agent = start(deckpilot_agent::system_clock(), |profile| {
        vec![
            steps(
                "On start",
                profile,
                WorkflowTrigger::lifecycle(LifecycleEvent::AgentStarted),
            ),
            steps(
                "On connect",
                profile,
                WorkflowTrigger::lifecycle(LifecycleEvent::DeviceConnected),
            ),
            steps(
                "On disconnect",
                profile,
                WorkflowTrigger::lifecycle(LifecycleEvent::DeviceDisconnected),
            ),
            steps(
                "On project",
                profile,
                WorkflowTrigger::lifecycle(LifecycleEvent::ProjectChanged),
            ),
            device_logger(profile),
        ]
    });
    let [on_start, on_connect, on_disconnect, on_project, logger] =
        std::array::from_fn(|i| agent.profile.workflows[i].id);
    // Nothing runs while locked.
    tokio::time::sleep(Duration::from_millis(300)).await;
    agent.unlock().await;

    // Once per agent start, when unlocked.
    let runs = agent.wait_for_runs(on_start, 1).await;
    assert_eq!(runs[0].summary.trigger, ExecutionTrigger::Lifecycle);
    assert_eq!(
        runs[0].summary.trigger_detail.as_deref(),
        Some("agentStarted")
    );
    // The device connected while locked is not an event.
    assert_eq!(agent.runs(on_connect).await.len(), 0);

    // A device is connected: both workflows start, the logger with the
    // device's serial as its input.
    agent.mock.plug(&models::stream_deck_mk2(), "SIM-0002");
    let runs = agent.wait_for_runs(logger, 1).await;
    assert_eq!(
        runs[0].summary.state,
        deckpilot_core::ExecutionState::Succeeded
    );
    let log = runs[0]
        .nodes
        .iter()
        .find(|n| n.node.as_str() == "log")
        .unwrap();
    assert_eq!(log.message, "connected SIM-0002");
    let runs = agent.wait_for_runs(on_connect, 1).await;
    assert_eq!(
        runs[0].summary.trigger_detail.as_deref(),
        Some("deviceConnected")
    );

    // The active project changes.
    agent
        .request(Request::CycleProject {
            direction: CycleDirection::Next,
        })
        .await;
    let runs = agent.wait_for_runs(on_project, 1).await;
    assert_eq!(
        runs[0].summary.trigger_detail.as_deref(),
        Some("projectChanged")
    );

    // Paused triggers do not start their workflow; others still do.
    agent
        .save(|profile| {
            profile
                .workflows
                .iter_mut()
                .find(|w| w.id == on_connect)
                .unwrap()
                .triggers_paused = true;
        })
        .await;
    let device = agent.mock.plug(&models::stream_deck_mk2(), "SIM-0003");
    agent.wait_for_runs(logger, 2).await;
    agent.mock.unplug(&device);
    agent.wait_for_runs(on_disconnect, 1).await;
    assert_eq!(agent.runs(on_connect).await.len(), 1);
    // The agent started once.
    assert_eq!(agent.runs(on_start).await.len(), 1);
    // Recorded in history with the trigger kind.
    let stored = agent
        .store()
        .execution(agent.runs(on_disconnect).await[0].summary.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.summary.trigger, ExecutionTrigger::Lifecycle);
    assert_eq!(
        stored.summary.trigger_detail.as_deref(),
        Some("deviceDisconnected")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn schedules_follow_the_agent_clock_only_while_unlocked() {
    let offset = Arc::new(AtomicU64::new(0));
    let clock: Clock = {
        let offset = offset.clone();
        Arc::new(move || SystemTime::now() + Duration::from_secs(offset.load(Ordering::SeqCst)))
    };
    let agent = start(clock, |profile| {
        vec![steps(
            "Every minute",
            profile,
            WorkflowTrigger::schedule(Schedule::Interval { minutes: 1 }),
        )]
    });
    let workflow = agent.profile.workflows[0].id;
    agent.unlock().await;
    // Counting starts when unlocked; nothing fires at once.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(agent.runs(workflow).await.len(), 0);

    offset.fetch_add(61, Ordering::SeqCst);
    let runs = agent.wait_for_runs(workflow, 1).await;
    assert_eq!(runs[0].summary.trigger, ExecutionTrigger::Schedule);
    assert_eq!(
        runs[0].summary.trigger_detail.as_deref(),
        Some("every 1 min")
    );

    // Locked: due times pass without runs, and are not made up after.
    agent.request(Request::Lock).await;
    offset.fetch_add(60, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    agent.unlock().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(agent.runs(workflow).await.len(), 1);
    // Counting starts again from the unlock.
    offset.fetch_add(61, Ordering::SeqCst);
    agent.wait_for_runs(workflow, 2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_firing_is_skipped_while_the_previous_run_still_runs() {
    let agent = start(deckpilot_agent::system_clock(), |_| {
        let mut slow = Workflow::new("Slow");
        slow.project_scope = Some(ProjectScope::Execution);
        slow.graph = Some(WorkflowGraph {
            version: GRAPH_VERSION,
            nodes: vec![
                node("start", NodeKind::Trigger),
                node("wait", NodeKind::Delay { ms: 1_500 }),
            ],
            edges: vec![deckpilot_core::graph::Edge {
                from: NodeId::new("start").unwrap(),
                port: deckpilot_core::graph::Port::Out,
                to: NodeId::new("wait").unwrap(),
            }],
        });
        slow.triggers
            .push(WorkflowTrigger::lifecycle(LifecycleEvent::DeviceConnected));
        vec![slow]
    });
    let workflow = agent.profile.workflows[0].id;
    agent.unlock().await;
    agent.mock.plug(&models::stream_deck_mk2(), "SIM-0002");
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while agent.runs(workflow).await.is_empty() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Still running: this connection does not start a second run.
    agent.mock.plug(&models::stream_deck_mk2(), "SIM-0003");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let runs = agent.wait_for_runs(workflow, 1).await;
    assert_eq!(
        runs[0].summary.state,
        deckpilot_core::ExecutionState::Succeeded
    );
}
