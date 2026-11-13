//! Reusable helpers for end-to-end agent tests: a temporary agent with
//! simulated devices and its own data directory, fake programs, a local
//! mock HTTP server and workflow builders. Nothing here touches real
//! repositories, containers, the network or applications.
//!
//! Each test binary uses a different subset, hence `dead_code`.
#![allow(dead_code)]

pub mod fakes;
pub mod http;
pub mod watchdog;

use std::path::{Path, PathBuf};
use std::time::Duration;

use deckpilot_agent::{Agent, AgentError, AgentOptions, Clock, InstanceLock};
use deckpilot_core::graph::WorkflowGraph;
use deckpilot_core::{
    ExecutionId, ExecutionRecord, ExecutionState, ExecutionSummary, ModuleSettings, Profile,
    ProfileId, Workflow, WorkflowId,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_ipc::{
    AgentClient, AgentPaths, AgentStatus, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Upper bound for conditions to be reached; generous for slow runners.
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// A test still running after this hangs: the test binary is ended.
pub const HANG: Duration = Duration::from_secs(180);
/// Longest a single request may take (workflow runs are answered when
/// they end).
pub const REQUEST_LIMIT: Duration = Duration::from_secs(60);

/// A directory removed when dropped.
#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", ProfileId::new()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An encrypted database in `paths`, as the configuration app creates it.
pub fn create_database(paths: &AgentPaths) -> DatabaseKey {
    Vault::new(&paths.database)
        .with_test_cost()
        .initialize(&Password::new("test password"))
        .unwrap()
}

pub fn open_store(paths: &AgentPaths, key: &DatabaseKey) -> SqliteStore {
    SqliteStore::open(&paths.database, &Encryption::Key(key.clone())).unwrap()
}

/// A data directory with an encrypted database holding `profile` as the
/// active profile, and module settings.
pub fn data_dir(
    dir: &TempDir,
    profile: &Profile,
    modules: &[(deckpilot_core::ModuleId, deckpilot_core::ActionParameters)],
) -> (AgentPaths, DatabaseKey) {
    let paths = AgentPaths::in_dir(dir.path().join("data"));
    paths.create_dirs().unwrap();
    let key = create_database(&paths);
    let store = open_store(&paths, &key);
    store.save_profile(profile).unwrap();
    store.set_active_profile(Some(profile.id)).unwrap();
    for (id, config) in modules {
        store
            .set_module_state(
                id,
                &ModuleSettings {
                    enabled: true,
                    config: config.clone(),
                },
            )
            .unwrap();
    }
    (paths, key)
}

/// A running agent with simulated devices.
pub struct TestAgent {
    pub paths: AgentPaths,
    pub key: DatabaseKey,
    pub mock: MockBackend,
    /// In-process access, e.g. to the work session.
    pub handle: deckpilot_agent::AgentHandle,
    task: Option<JoinHandle<Result<(), AgentError>>>,
    stop: Option<oneshot::Sender<()>>,
    /// Ends the test binary if the test hangs.
    _watchdog: watchdog::Watchdog,
}

impl Drop for TestAgent {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl TestAgent {
    /// Starts an agent on `paths` (locked, as at login). `clock` replaces
    /// the system clock, e.g. to move time for schedules.
    pub fn start(
        paths: &AgentPaths,
        key: &DatabaseKey,
        mock: &MockBackend,
        clock: Option<Clock>,
    ) -> Self {
        Self::start_with_gestures(paths, key, mock, clock, None)
    }

    /// [`TestAgent::start`] with keys put in gesture mode by `gestures`.
    pub fn start_with_gestures(
        paths: &AgentPaths,
        key: &DatabaseKey,
        mock: &MockBackend,
        clock: Option<Clock>,
        gestures: Option<std::sync::Arc<dyn deckpilot_agent::GestureHooks>>,
    ) -> Self {
        Self::start_with_backend(paths, key, mock, Box::new(mock.clone()), clock, gestures)
    }

    /// [`TestAgent::start_with_gestures`] with the devices of `backend`, e.g.
    /// a driver defined by a test combined with `mock`.
    pub fn start_with_backend(
        paths: &AgentPaths,
        key: &DatabaseKey,
        mock: &MockBackend,
        backend: Box<dyn deckpilot_device::DeviceBackend>,
        clock: Option<Clock>,
        gestures: Option<std::sync::Arc<dyn deckpilot_agent::GestureHooks>>,
    ) -> Self {
        let lock = InstanceLock::acquire_waiting(&paths.lock_file, Duration::from_secs(2)).unwrap();
        let options = AgentOptions {
            paths: paths.clone(),
            backend,
            mock: Some(mock.clone()),
            rescan_interval: Duration::from_millis(50),
            autostart: None,
            open_desktop: None,
            open_window: None,
            key_store: None,
            auto_lock_check: Duration::from_millis(50),
            cancel_grace: Duration::from_secs(10),
            clock: clock.unwrap_or_else(deckpilot_agent::system_clock),
            notifier: None,
            screen_lock: None,
        };
        let mut agent = Agent::start(options, lock).unwrap();
        if let Some(gestures) = gestures {
            agent = agent.with_gesture_hooks(gestures);
        }
        let handle = agent.handle();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(agent.run(async move {
            let _ = stopped.await;
        }));
        Self {
            paths: paths.clone(),
            key: key.clone(),
            mock: mock.clone(),
            handle,
            task: Some(task),
            stop: Some(stop),
            _watchdog: watchdog::Watchdog::new(
                std::thread::current().name().unwrap_or("a test"),
                HANG,
            ),
        }
    }

    /// Starts an agent and unlocks it the way the configuration app does.
    pub async fn start_unlocked(
        paths: &AgentPaths,
        key: &DatabaseKey,
        mock: &MockBackend,
        clock: Option<Clock>,
    ) -> Self {
        let agent = Self::start(paths, key, mock, clock);
        agent.unlock().await;
        agent
    }

    pub async fn unlock(&self) {
        self.request(Request::Unlock {
            key: KeyMaterial::new(self.key.to_hex()),
        })
        .await
        .unwrap();
    }

    pub async fn client(&self) -> AgentClient {
        AgentClient::connect(&self.paths, ClientKind::Desktop)
            .await
            .unwrap()
    }

    /// A client subscribed to the agent's events.
    pub async fn events(&self) -> AgentClient {
        let mut client = self.client().await;
        client.subscribe().await.unwrap();
        client
    }

    pub async fn request(&self, request: Request) -> Result<Response, ClientError> {
        let what = format!("the {} request", request.name());
        watchdog::within(&what, REQUEST_LIMIT, async {
            self.client().await.request(request).await
        })
        .await
    }

    pub async fn status(&self) -> AgentStatus {
        match self.request(Request::GetStatus).await.unwrap() {
            Response::Status { status } => *status,
            other => panic!("unexpected {other:?}"),
        }
    }

    pub fn store(&self) -> SqliteStore {
        open_store(&self.paths, &self.key)
    }

    /// Changes the stored active profile and makes the agent reload it.
    pub async fn edit_profile(&self, id: ProfileId, change: impl FnOnce(&mut Profile)) {
        let store = self.store();
        let mut profile = store.load_profile(id).unwrap().unwrap();
        change(&mut profile);
        store.save_profile(&profile).unwrap();
        self.request(Request::ReloadConfiguration).await.unwrap();
    }

    pub async fn start_workflow(
        &self,
        workflow: WorkflowId,
    ) -> Result<ExecutionSummary, ClientError> {
        match self.request(Request::StartWorkflow { workflow }).await? {
            Response::ExecutionStarted { execution } => Ok(execution),
            other => panic!("unexpected {other:?}"),
        }
    }

    pub async fn execution(&self, execution: ExecutionId) -> ExecutionRecord {
        match self
            .request(Request::GetExecution { execution })
            .await
            .unwrap()
        {
            Response::Execution { execution } => *execution,
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Runs of `workflow`, newest first.
    pub async fn runs(&self, workflow: WorkflowId) -> Vec<ExecutionSummary> {
        match self
            .request(Request::ListExecutions {
                workflow: Some(workflow),
                limit: None,
            })
            .await
            .unwrap()
        {
            Response::Executions { executions } => executions,
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Waits until `predicate` holds for the runs of `workflow`.
    pub async fn wait_for_runs(
        &self,
        workflow: WorkflowId,
        predicate: impl Fn(&[ExecutionSummary]) -> bool,
    ) -> Vec<ExecutionSummary> {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let runs = self.runs(workflow).await;
            if predicate(&runs) {
                return runs;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "condition not reached: {runs:#?}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Waits until the run has ended and returns its record.
    pub async fn wait_finished(&self, execution: ExecutionId) -> ExecutionRecord {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let record = self.execution(execution).await;
            if record.summary.state != ExecutionState::Running {
                return record;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "still running: {record:#?}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Waits until the node `node` of the run is running.
    pub async fn wait_for_node(&self, execution: ExecutionId, node: &str) {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let record = self.execution(execution).await;
            if record
                .summary
                .running_nodes
                .iter()
                .any(|n| n.as_str() == node)
            {
                return;
            }
            assert!(
                record.summary.state == ExecutionState::Running
                    && tokio::time::Instant::now() < deadline,
                "`{node}` did not run: {record:#?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until the node `node` of the run has finished (is in its
    /// record), while the run may still go on.
    pub async fn wait_for_finished_node(&self, execution: ExecutionId, node: &str) {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let record = self.execution(execution).await;
            if record.nodes.iter().any(|n| n.node.as_str() == node) {
                return;
            }
            assert!(
                record.summary.state == ExecutionState::Running
                    && tokio::time::Instant::now() < deadline,
                "`{node}` did not finish: {record:#?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Asks the agent to stop and waits until it has.
    pub async fn shutdown(mut self) {
        self.request(Request::Shutdown).await.unwrap();
        let task = self.task.take().unwrap();
        tokio::time::timeout(TIMEOUT, task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        self.stop = None;
    }
}

/// A graph from JSON nodes and edges, in the stored format.
pub fn graph(nodes: &serde_json::Value, edges: &serde_json::Value) -> WorkflowGraph {
    serde_json::from_value(serde_json::json!({"version": 1, "nodes": nodes, "edges": edges}))
        .expect("a valid graph")
}

/// A workflow named `name` that runs `graph`, keeping project changes in
/// its runs.
pub fn graph_workflow(name: &str, graph: WorkflowGraph) -> Workflow {
    let mut workflow = Workflow::new(name);
    workflow.project_scope = Some(deckpilot_core::ProjectScope::Execution);
    workflow.graph = Some(graph);
    workflow
}

/// `start → wait (ms) → done`: a workflow that runs for a while and can be
/// cancelled at once.
pub fn waiting_workflow(name: &str, ms: u32) -> Workflow {
    graph_workflow(
        name,
        graph(
            &serde_json::json!([
                {"id": "start", "type": "trigger"},
                {"id": "wait", "type": "delay", "ms": ms},
            ]),
            &serde_json::json!([{"from": "start", "to": "wait"}]),
        ),
    )
}

/// The action registry of the built-in modules.
pub fn registry() -> deckpilot_core::ActionRegistry {
    deckpilot_modules::builtin(deckpilot_core::Os::current()).action_registry()
}
