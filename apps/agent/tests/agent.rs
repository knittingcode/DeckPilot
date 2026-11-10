//! End-to-end tests of the agent with simulated devices and real IPC.

use std::time::Duration;

use deckpilot_agent::{Agent, AgentError, AgentOptions, InstanceLock, LockError};
use deckpilot_core::{
    ActionParameters, ActionRegistry, ActionStatus, CycleDirection, DeviceLayout, LayoutStatus,
    ModuleSettings, Os, Profile, ProfileId, Project, ProjectKey,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, models};
use deckpilot_ipc::framing::{read_message, write_message};
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, AgentStatus, ClientError, ClientKind, ErrorCode,
    PROTOCOL_VERSION, Request, Response, ServerMessage, SharedStream,
};
use deckpilot_modules::projects::{ProjectCatalog, actions};
use deckpilot_modules::{general, projects};
use deckpilot_storage::{ConfigStore, Encryption, SqliteStore};
use serde_json::{Value, json};
use tokio::io::BufReader;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const TIMEOUT: Duration = Duration::from_secs(5);

/// The agent task and the sender that stops it.
type Running = (JoinHandle<Result<(), AgentError>>, oneshot::Sender<()>);

struct TestAgent {
    paths: AgentPaths,
    mock: MockBackend,
    device: DeviceId,
    work: Profile,
    task: Option<JoinHandle<Result<(), AgentError>>>,
    stop: Option<oneshot::Sender<()>>,
}

impl Drop for TestAgent {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

fn temp_paths() -> AgentPaths {
    let dir = std::env::temp_dir().join(format!("deckpilot-agent-test-{}", ProfileId::new()));
    let paths = AgentPaths::in_dir(dir);
    paths.create_dirs().unwrap();
    paths
}

/// A profile with SHOP, CMR and CMS, a project switcher on key 0, a log
/// action on key 1 and a generated MK.2 layout.
fn seed(paths: &AgentPaths) -> Profile {
    let store = SqliteStore::open(&paths.database, &Encryption::None).unwrap();
    let registry = registry();
    let mut profile = Profile::new("Work");
    for key in ["SHOP", "CMR", "CMS"] {
        profile
            .add_project(Project::new(ProjectKey::new(key).unwrap(), key))
            .unwrap();
    }
    profile.actions.push(
        registry
            .create_instance("Next project", actions::next_id(), &ActionParameters::new())
            .unwrap(),
    );
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
    let layout = DeviceLayout::generate("Main", &models::stream_deck_mk2(), &profile);
    profile.layouts.push(layout);
    store.save_profile(&profile).unwrap();
    store.set_active_profile(Some(profile.id)).unwrap();
    profile
}

fn registry() -> ActionRegistry {
    deckpilot_modules::builtin(Os::current()).action_registry()
}

fn start_agent(paths: AgentPaths, mock: MockBackend) -> Result<Running, AgentError> {
    let lock = InstanceLock::acquire(&paths.lock_file)?;
    let options = AgentOptions {
        paths,
        backend: Box::new(mock.clone()),
        mock: Some(mock),
        rescan_interval: Duration::from_millis(50),
        autostart: None,
        open_desktop: None,
    };
    let agent = Agent::start(options, lock)?;
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(agent.run(async move {
        let _ = stopped.await;
    }));
    Ok((task, stop))
}

async fn setup() -> TestAgent {
    let paths = temp_paths();
    let work = seed(&paths);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "TEST-0001");
    let (task, stop) = start_agent(paths.clone(), mock.clone()).unwrap();
    let agent = TestAgent {
        paths,
        mock,
        device,
        work,
        task: Some(task),
        stop: Some(stop),
    };
    agent
        .wait_for_status(|s| {
            s.devices
                .iter()
                .any(|d| d.state == ConnectionState::Connected)
        })
        .await;
    agent
}

impl TestAgent {
    async fn client(&self) -> AgentClient {
        AgentClient::connect(&self.paths, ClientKind::Other)
            .await
            .unwrap()
    }

    async fn status(&self) -> AgentStatus {
        match self
            .client()
            .await
            .request(Request::GetStatus)
            .await
            .unwrap()
        {
            Response::Status { status } => *status,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn wait_for_status(&self, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            if let Ok(mut client) = AgentClient::connect(&self.paths, ClientKind::Other).await
                && let Ok(Response::Status { status }) = client.request(Request::GetStatus).await
                && predicate(&status)
            {
                return *status;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "condition not reached in time"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn store(&self) -> SqliteStore {
        SqliteStore::open(&self.paths.database, &Encryption::None).unwrap()
    }

    fn frames_shown(&self) -> bool {
        self.mock
            .wait_for(&self.device, TIMEOUT, |frames, _| !frames.is_empty())
    }

    async fn shutdown(mut self) {
        self.client()
            .await
            .request(Request::Shutdown)
            .await
            .unwrap();
        let task = self.task.take().unwrap();
        tokio::time::timeout(TIMEOUT, task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

fn active_key(status: &AgentStatus) -> Option<&str> {
    status.active_project.as_ref().map(|p| p.key.as_str())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn renders_layout_and_reports_status() {
    let agent = setup().await;
    assert!(agent.frames_shown(), "layout should be rendered on connect");
    let frames = agent.mock.frames(&agent.device);
    // Key 0 (project switcher) and key 1 (log action) are bound; the rest are blank.
    assert!(frames.contains_key(&ControlId::Key { index: 0 }));
    assert!(frames.contains_key(&ControlId::Key { index: 1 }));
    assert!(!frames.contains_key(&ControlId::Key { index: 2 }));
    assert_eq!(frames[&ControlId::Key { index: 0 }].width(), 72);

    let status = agent.status().await;
    assert_eq!(
        status.active_profile.as_ref().map(|p| p.name.as_str()),
        Some("Work")
    );
    assert_eq!(active_key(&status), Some("SHOP"));
    assert_eq!(status.backend, "mock");
    assert_eq!(status.protocol, PROTOCOL_VERSION);
    let device = &status.devices[0];
    assert_eq!(device.model_name, "Stream Deck MK.2");
    assert!(matches!(device.layout, LayoutStatus::Compatible { .. }));
    assert_eq!(device.page.as_deref(), Some("Home"));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn button_press_cycles_project_persists_and_notifies() {
    let agent = setup().await;
    assert!(agent.frames_shown());
    let mut subscriber = agent.client().await;
    subscriber.subscribe().await.unwrap();
    let writes_before = agent.mock.image_writes(&agent.device);

    // A tap is a press and a release; only the press is bound.
    agent.mock.tap(&agent.device, ControlId::Key { index: 0 });

    let mut completed = 0;
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    let status = loop {
        let event = tokio::time::timeout_at(deadline, subscriber.next_event())
            .await
            .unwrap()
            .unwrap();
        match event {
            AgentEvent::ActionCompleted { name, status, .. } => {
                assert_eq!(name, "Next project");
                assert_eq!(status, ActionStatus::Succeeded);
                completed += 1;
            }
            AgentEvent::State { status } if active_key(&status) == Some("CMR") => break status,
            _ => {}
        }
    };
    assert_eq!(completed, 1);
    assert_eq!(active_key(&status), Some("CMR"));
    let stored = agent.store().active_project(agent.work.id).unwrap();
    assert_eq!(stored, Some(agent.work.projects[1].id));

    // The switcher key shows the project, so it is redrawn; unchanged keys are not.
    assert!(
        agent
            .mock
            .wait_for(&agent.device, TIMEOUT, |_, writes| writes > writes_before)
    );
    let redrawn = agent.mock.image_writes(&agent.device) - writes_before;
    assert_eq!(redrawn, 1, "only the project switcher key changes");

    // CMR -> CMS -> SHOP
    for expected in ["CMS", "SHOP"] {
        agent.mock.tap(&agent.device, ControlId::Key { index: 0 });
        agent
            .wait_for_status(|s| active_key(s) == Some(expected))
            .await;
    }
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switches_profiles_without_restart() {
    let agent = setup().await;
    assert!(agent.frames_shown());
    let home = Profile::new("Home");
    agent.store().save_profile(&home).unwrap();

    let mut client = agent.client().await;
    let Response::Profiles { profiles, active } =
        client.request(Request::ListProfiles).await.unwrap()
    else {
        panic!("expected profiles")
    };
    assert_eq!(profiles.len(), 2);
    assert_eq!(active, Some(agent.work.id));

    let response = client
        .request(Request::SetActiveProfile { profile: home.id })
        .await
        .unwrap();
    assert!(matches!(response, Response::ActiveProfile { profile: Some(p) } if p.id == home.id));
    assert_eq!(agent.store().active_profile().unwrap(), Some(home.id));

    // Home has no layout: the device shows the fallback notice only.
    assert!(
        agent
            .mock
            .wait_for(&agent.device, TIMEOUT, |frames, _| frames.len() == 1)
    );
    let status = agent.status().await;
    assert_eq!(status.devices[0].layout, LayoutStatus::Missing);
    assert_eq!(status.active_project, None);

    let missing = client
        .request(Request::SetActiveProfile {
            profile: ProfileId::new(),
        })
        .await;
    assert!(matches!(missing, Err(ClientError::Agent(e)) if e.code == ErrorCode::NotFound));

    client
        .request(Request::SetActiveProfile {
            profile: agent.work.id,
        })
        .await
        .unwrap();
    assert!(
        agent
            .mock
            .wait_for(&agent.device, TIMEOUT, |frames, _| frames.len() == 2)
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_and_action_requests() {
    let agent = setup().await;
    let mut client = agent.client().await;
    let response = client
        .request(Request::CycleProject {
            direction: CycleDirection::Previous,
        })
        .await
        .unwrap();
    assert!(
        matches!(response, Response::ActiveProject { project: Some(p) } if p.key.as_str() == "CMS")
    );

    let cmr = agent.work.projects[1].id;
    client
        .request(Request::SelectProject { project: cmr })
        .await
        .unwrap();
    let Response::ActiveProject { project } =
        client.request(Request::GetActiveProject).await.unwrap()
    else {
        panic!("expected project")
    };
    assert_eq!(project.unwrap().id, cmr);

    let announce = agent.work.actions[1].id;
    let Response::ActionResult { result } = client
        .request(Request::RunAction { action: announce })
        .await
        .unwrap()
    else {
        panic!("expected action result")
    };
    assert_eq!(result.message, "CMR");

    // Simulated input goes through the same path as hardware input.
    client
        .request(Request::SimulateInput {
            device: agent.device.clone(),
            control: ControlId::Key { index: 0 },
            input: ControlInput::Press,
        })
        .await
        .unwrap();
    agent
        .wait_for_status(|s| active_key(s) == Some("CMS"))
        .await;

    let unknown = client
        .request(Request::SelectProject {
            project: deckpilot_core::ProjectId::new(),
        })
        .await;
    assert!(matches!(unknown, Err(ClientError::Agent(e)) if e.code == ErrorCode::NotFound));
    let autostart = client.request(Request::GetAutostart).await;
    assert!(matches!(autostart, Err(ClientError::Agent(e)) if e.code == ErrorCode::Unsupported));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovers_from_unplug_and_replug() {
    let agent = setup().await;
    assert!(agent.frames_shown());
    agent.mock.unplug(&agent.device);
    let status = agent
        .wait_for_status(|s| {
            s.devices
                .iter()
                .any(|d| d.state == ConnectionState::Disconnected)
        })
        .await;
    assert_eq!(status.devices[0].page, None);

    // Plugging resets the simulated display; the agent must redraw it.
    agent
        .mock
        .plug(&models::stream_deck_mk2(), &agent.device.serial);
    agent
        .wait_for_status(|s| {
            s.devices
                .iter()
                .any(|d| d.state == ConnectionState::Connected)
        })
        .await;
    assert!(agent.frames_shown(), "layout restored after reconnect");

    // Input works again on the new connection.
    agent.mock.tap(&agent.device, ControlId::Key { index: 0 });
    agent
        .wait_for_status(|s| active_key(s) == Some("CMR"))
        .await;
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn brightness_is_applied_and_validated() {
    let agent = setup().await;
    let mut client = agent.client().await;
    client
        .request(Request::SetBrightness { percent: 30 })
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + TIMEOUT;
    while agent.mock.brightness(&agent.device) != Some(30) {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(agent.store().device_brightness().unwrap(), Some(30));
    let invalid = client
        .request(Request::SetBrightness { percent: 101 })
        .await;
    assert!(matches!(invalid, Err(ClientError::Agent(e)) if e.code == ErrorCode::InvalidRequest));
    agent.shutdown().await;
}

/// Sends raw JSON lines and returns the replies, bypassing the typed client.
struct RawConnection {
    reader: BufReader<SharedStream>,
    writer: SharedStream,
}

impl RawConnection {
    async fn open(paths: &AgentPaths) -> Self {
        let stream = SharedStream::new(paths.endpoint().connect().await.unwrap());
        Self {
            reader: BufReader::new(stream.clone()),
            writer: stream,
        }
    }

    async fn send(&mut self, value: &Value) {
        write_message(&mut self.writer, value).await.unwrap();
    }

    async fn send_line(&mut self, line: &str) {
        use tokio::io::AsyncWriteExt;
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn receive(&mut self) -> Option<ServerMessage> {
        tokio::time::timeout(TIMEOUT, read_message(&mut self.reader))
            .await
            .unwrap()
            .ok()
            .flatten()
    }

    fn error_code(message: Option<ServerMessage>) -> ErrorCode {
        match message {
            Some(ServerMessage::Error { error, .. }) => error.code,
            other => panic!("expected an error, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejects_unauthenticated_and_invalid_messages() {
    let agent = setup().await;
    let token = std::fs::read_to_string(&agent.paths.token_file).unwrap();

    let mut wrong_token = RawConnection::open(&agent.paths).await;
    wrong_token
        .send(&json!({"type":"hello","protocol":PROTOCOL_VERSION,"token":"nope","client":"other"}))
        .await;
    assert_eq!(
        RawConnection::error_code(wrong_token.receive().await),
        ErrorCode::Unauthorized
    );
    assert!(
        wrong_token.receive().await.is_none(),
        "connection must be closed"
    );

    let mut old_protocol = RawConnection::open(&agent.paths).await;
    old_protocol
        .send(&json!({"type":"hello","protocol":0,"token":token,"client":"other"}))
        .await;
    assert_eq!(
        RawConnection::error_code(old_protocol.receive().await),
        ErrorCode::UnsupportedProtocol
    );

    let mut no_hello = RawConnection::open(&agent.paths).await;
    no_hello
        .send(&json!({"type":"request","id":1,"request":{"type":"getStatus"}}))
        .await;
    assert_eq!(
        RawConnection::error_code(no_hello.receive().await),
        ErrorCode::Unauthorized
    );

    let mut client = RawConnection::open(&agent.paths).await;
    client
        .send(&json!({"type":"hello","protocol":PROTOCOL_VERSION,"token":token,"client":"other"}))
        .await;
    assert!(matches!(
        client.receive().await,
        Some(ServerMessage::Welcome { .. })
    ));

    client.send_line("this is not json").await;
    assert_eq!(
        RawConnection::error_code(client.receive().await),
        ErrorCode::InvalidRequest
    );
    client
        .send(&json!({"type":"request","id":2,"request":{"type":"formatDisk"}}))
        .await;
    let reply = client.receive().await;
    assert!(
        matches!(&reply, Some(ServerMessage::Error { id: Some(2), .. })),
        "{reply:?}"
    );
    client
        .send(&json!({"type":"request","id":3,"request":{"type":"setBrightness","percent":101}}))
        .await;
    assert_eq!(
        RawConnection::error_code(client.receive().await),
        ErrorCode::InvalidRequest
    );

    // The connection stays usable after rejected requests.
    client
        .send(&json!({"type":"request","id":4,"request":{"type":"getActiveProfile"}}))
        .await;
    assert!(matches!(
        client.receive().await,
        Some(ServerMessage::Response { id: 4, .. })
    ));

    // Oversized messages close the connection.
    client
        .send_line(&"x".repeat(deckpilot_ipc::framing::MAX_MESSAGE_BYTES + 1))
        .await;
    assert_eq!(
        RawConnection::error_code(client.receive().await),
        ErrorCode::InvalidRequest
    );
    assert!(client.receive().await.is_none());
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_instance_shutdown_and_client_reconnect() {
    let mut agent = setup().await;
    let paths = agent.paths.clone();
    assert!(matches!(
        InstanceLock::acquire(&paths.lock_file),
        Err(LockError::AlreadyRunning)
    ));
    assert!(matches!(
        start_agent(paths.clone(), MockBackend::new()),
        Err(AgentError::Lock(LockError::AlreadyRunning))
    ));

    let mut subscriber = agent.client().await;
    subscriber.subscribe().await.unwrap();
    assert!(agent.frames_shown());

    agent
        .client()
        .await
        .request(Request::Shutdown)
        .await
        .unwrap();
    let task = agent.task.take().unwrap();
    tokio::time::timeout(TIMEOUT, task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // Subscribers are told before the connection closes.
    let mut saw_shutdown = false;
    while let Ok(Ok(event)) = tokio::time::timeout(TIMEOUT, subscriber.next_event()).await {
        if event == AgentEvent::ShuttingDown {
            saw_shutdown = true;
            break;
        }
    }
    assert!(saw_shutdown);
    assert!(!paths.token_file.exists(), "token is removed on shutdown");
    assert!(
        agent
            .mock
            .wait_for(&agent.device, TIMEOUT, |frames, _| frames.is_empty()),
        "keys are blanked"
    );
    assert!(matches!(
        AgentClient::connect(&paths, ClientKind::Other).await,
        Err(ClientError::NotRunning)
    ));

    // A restarted agent accepts clients again and restores the selection.
    let (task, stop) = start_agent(paths.clone(), agent.mock.clone()).unwrap();
    agent.task = Some(task);
    agent.stop = Some(stop);
    let status = agent.wait_for_status(|s| !s.devices.is_empty()).await;
    assert_eq!(status.active_profile.unwrap().id, agent.work.id);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runs_workflows_over_ipc() {
    let agent = setup().await;
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let workflow = deckpilot_core::Workflow::new("Switch and announce")
        .step(profile.actions[0].id, deckpilot_core::OnFailure::Stop)
        .step(profile.actions[1].id, deckpilot_core::OnFailure::Stop);
    let workflow_id = workflow.id;
    profile.workflows.push(workflow);
    agent.store().save_profile(&profile).unwrap();

    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();
    let Response::WorkflowReport { report } = client
        .request(Request::RunWorkflow {
            workflow: workflow_id,
        })
        .await
        .unwrap()
    else {
        panic!("expected a workflow report")
    };
    assert_eq!(report.status, ActionStatus::Succeeded);
    assert_eq!(report.steps[1].result.message, "CMR");
    agent
        .wait_for_status(|s| active_key(s) == Some("CMR"))
        .await;
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configuration_reload_updates_the_device_without_restart() {
    let agent = setup().await;
    assert!(agent.frames_shown());
    let before = agent.mock.frames(&agent.device);

    // Rebind key 5 to the log action, as the layout designer would.
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let action = profile.actions[1].id;
    profile.layouts[0].pages[0]
        .bindings
        .push(deckpilot_core::ControlBinding {
            control: ControlId::Key { index: 5 },
            trigger: deckpilot_core::Trigger::Press,
            target: deckpilot_core::BindingTarget::Action { action },
            label: Some("{project.name}".into()),
        });
    agent.store().save_profile(&profile).unwrap();
    agent
        .client()
        .await
        .request(Request::ReloadConfiguration)
        .await
        .unwrap();

    let key = ControlId::Key { index: 5 };
    assert!(agent.mock.wait_for(&agent.device, TIMEOUT, |frames, _| {
        frames.contains_key(&key)
    }));
    let after = agent.mock.frames(&agent.device);
    // Keys 0 and 1 are unchanged and were not redrawn.
    assert_eq!(
        after[&ControlId::Key { index: 0 }],
        before[&ControlId::Key { index: 0 }]
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn renders_simulated_xl_and_plus_devices() {
    let paths = temp_paths();
    let store = SqliteStore::open(&paths.database, &Encryption::None).unwrap();
    let registry = registry();
    let mut profile = Profile::new("Studio");
    for _ in 0..3 {
        profile.actions.push(
            registry
                .create_instance("Next", actions::next_id(), &ActionParameters::new())
                .unwrap(),
        );
    }
    profile.layouts.push(DeviceLayout::generate(
        "XL",
        &models::stream_deck_xl(),
        &profile,
    ));
    profile.layouts.push(DeviceLayout::generate(
        "Plus",
        &models::stream_deck_plus(),
        &profile,
    ));
    store.save_profile(&profile).unwrap();
    store.set_active_profile(Some(profile.id)).unwrap();
    drop(store);

    let mock = MockBackend::new();
    let xl = mock.plug(&models::stream_deck_xl(), "SIM-XL");
    let plus = mock.plug(&models::stream_deck_plus(), "SIM-PLUS");
    let (task, stop) = start_agent(paths.clone(), mock.clone()).unwrap();
    assert!(mock.wait_for(&xl, TIMEOUT, |frames, _| frames.len() == 3));
    assert!(mock.wait_for(&plus, TIMEOUT, |frames, _| frames.len() == 3));
    assert_eq!(mock.frames(&xl)[&ControlId::Key { index: 0 }].width(), 96);
    assert_eq!(
        mock.frames(&plus)[&ControlId::Key { index: 0 }].width(),
        120
    );

    let mut client = AgentClient::connect(&paths, ClientKind::Other)
        .await
        .unwrap();
    let Response::Devices { devices } = client.request(Request::ListDevices).await.unwrap() else {
        panic!("expected devices")
    };
    assert!(devices.iter().all(|d| d.source == "mock"));
    assert!(
        devices
            .iter()
            .all(|d| matches!(d.layout, LayoutStatus::Compatible { .. }))
    );
    client.request(Request::Shutdown).await.unwrap();
    tokio::time::timeout(TIMEOUT, task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(stop);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previous_protocol_clients_can_stop_the_agent_for_an_update() {
    let mut agent = setup().await;
    assert!(agent.frames_shown());
    // A client from the previous release speaks protocol 1.
    let mut old = AgentClient::connect_with_protocol(&agent.paths, ClientKind::Desktop, 1)
        .await
        .unwrap();
    assert!(matches!(
        old.request(Request::GetStatus).await.unwrap(),
        Response::Status { .. }
    ));

    let mut client = agent.client().await;
    client.request(Request::PrepareForUpdate).await.unwrap();
    let task = agent.task.take().unwrap();
    tokio::time::timeout(TIMEOUT, task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        agent
            .mock
            .wait_for(&agent.device, TIMEOUT, |frames, _| frames.is_empty())
    );
    assert!(matches!(
        AgentClient::connect(&agent.paths, ClientKind::Other).await,
        Err(ClientError::NotRunning)
    ));
}

async fn run_action(
    client: &mut AgentClient,
    action: deckpilot_core::ActionInstanceId,
) -> deckpilot_core::ActionResult {
    match client.request(Request::RunAction { action }).await.unwrap() {
        Response::ActionResult { result } => result,
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_modules_stop_their_actions_without_losing_them() {
    let agent = setup().await;
    let mut client = agent.client().await;
    let next = agent.work.actions[0].id;
    assert!(run_action(&mut client, next).await.is_success());

    // The configuration app disables the module and asks for a reload.
    agent
        .store()
        .set_module_state(
            &projects::module_id(),
            &ModuleSettings {
                enabled: false,
                config: ActionParameters::new(),
            },
        )
        .unwrap();
    client.request(Request::ReloadConfiguration).await.unwrap();
    let result = run_action(&mut client, next).await;
    assert_eq!(result.status, ActionStatus::Failed);
    assert_eq!(
        result.message,
        "this action is unavailable: the Projects module is disabled"
    );
    let cycle = client
        .request(Request::CycleProject {
            direction: CycleDirection::Next,
        })
        .await;
    assert!(matches!(cycle, Err(ClientError::Agent(e)) if e.message.contains("disabled")));
    // The stored action is untouched.
    let stored = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    assert_eq!(stored.actions, agent.work.actions);
    assert_eq!(active_key(&agent.status().await), Some("CMR"));

    agent
        .store()
        .set_module_state(
            &projects::module_id(),
            &ModuleSettings {
                enabled: true,
                config: ActionParameters::new(),
            },
        )
        .unwrap();
    client.request(Request::ReloadConfiguration).await.unwrap();
    assert!(run_action(&mut client, next).await.is_success());
    assert_eq!(active_key(&agent.status().await), Some("CMS"));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profiles_keep_their_own_projects_and_selection() {
    let agent = setup().await;
    let mut home = Profile::new("Home");
    for key in ["BLOG", "GARDEN"] {
        home.add_project(Project::new(ProjectKey::new(key).unwrap(), key))
            .unwrap();
    }
    agent.store().save_profile(&home).unwrap();
    let mut client = agent.client().await;
    let cms = agent.work.projects[2].id;
    client
        .request(Request::SelectProject { project: cms })
        .await
        .unwrap();

    client
        .request(Request::SetActiveProfile { profile: home.id })
        .await
        .unwrap();
    assert_eq!(active_key(&agent.status().await), Some("BLOG"));
    // A project of the inactive profile cannot be selected.
    let foreign = client
        .request(Request::SelectProject { project: cms })
        .await;
    assert!(matches!(foreign, Err(ClientError::Agent(e)) if e.code == ErrorCode::NotFound));
    assert_eq!(active_key(&agent.status().await), Some("BLOG"));

    client
        .request(Request::SetActiveProfile {
            profile: agent.work.id,
        })
        .await
        .unwrap();
    assert_eq!(active_key(&agent.status().await), Some("CMS"));
    agent.shutdown().await;
}
