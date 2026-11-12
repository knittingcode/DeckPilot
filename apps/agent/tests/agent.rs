//! End-to-end tests of the agent with simulated devices and real IPC.

use std::time::Duration;

use deckpilot_agent::{Agent, AgentError, AgentOptions, Clock, InstanceLock, LockError};
use deckpilot_core::{
    ActionParameters, ActionRegistry, ActionStatus, CycleDirection, DeviceLayout, LayoutStatus,
    ModuleSettings, Os, Profile, ProfileId, Project, ProjectKey,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, models};
use deckpilot_ipc::framing::{read_message, write_message};
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, AgentStatus, ClientError, ClientKind, ErrorCode,
    KeyMaterial, LockReason, PROTOCOL_VERSION, ProtectionMode, Request, Response, SecurityState,
    ServerMessage, SharedStream,
};
use deckpilot_keystore::MemoryKeyStore;
use deckpilot_modules::projects::{ProjectCatalog, actions};
use deckpilot_modules::{general, projects};
use deckpilot_presence::{ScreenLockMonitor, ScriptedScreenLock};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore};
use serde_json::{Value, json};
use tokio::io::BufReader;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Upper bound for conditions to be reached; generous for slow CI runners
/// (passing tests continue as soon as the condition holds).
const TIMEOUT: Duration = Duration::from_secs(20);

/// The agent task and the sender that stops it.
type Running = (JoinHandle<Result<(), AgentError>>, oneshot::Sender<()>);

struct TestAgent {
    paths: AgentPaths,
    mock: MockBackend,
    device: DeviceId,
    work: Profile,
    key: DatabaseKey,
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
/// An encrypted database as the configuration app creates it on first run.
fn create_database(paths: &AgentPaths) -> (SqliteStore, DatabaseKey) {
    let key = Vault::new(&paths.database)
        .with_test_cost()
        .initialize(&Password::new("test password"))
        .unwrap();
    let store = SqliteStore::open(&paths.database, &Encryption::Key(key.clone())).unwrap();
    (store, key)
}

/// Unlocks a running agent the way the configuration app does.
async fn unlock_agent(paths: &AgentPaths, key: &DatabaseKey) {
    let mut client = AgentClient::connect(paths, ClientKind::Desktop)
        .await
        .unwrap();
    client
        .request(Request::Unlock {
            key: KeyMaterial::new(key.to_hex()),
        })
        .await
        .unwrap();
}

fn seed(paths: &AgentPaths) -> (Profile, DatabaseKey) {
    let (store, key) = create_database(paths);
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
    (profile, key)
}

fn registry() -> ActionRegistry {
    deckpilot_modules::builtin(Os::current()).action_registry()
}

fn start_agent(paths: AgentPaths, mock: MockBackend) -> Result<Running, AgentError> {
    start_agent_with(paths, mock, Extras::default())
}

/// Optional test doubles. Without `keys` the agent has no credential store.
#[derive(Default)]
struct Extras {
    opened: Option<Opened>,
    keys: Option<MemoryKeyStore>,
    clock: Option<Clock>,
    screen: Option<std::sync::Arc<ScriptedScreenLock>>,
    /// Native notifications; never real ones in tests.
    notifier: Option<std::sync::Arc<deckpilot_core::notify::RecordingNotifier>>,
    /// How long a cancelled run may take to stop (default 10 s).
    cancel_grace: Option<Duration>,
}

/// Windows the agent asked to open by starting the app.
type Opened = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

fn start_agent_with(
    paths: AgentPaths,
    mock: MockBackend,
    extras: Extras,
) -> Result<Running, AgentError> {
    let Extras {
        opened,
        keys,
        clock,
        screen,
        notifier,
        cancel_grace,
    } = extras;
    let lock = InstanceLock::acquire(&paths.lock_file)?;
    let options = AgentOptions {
        paths,
        backend: Box::new(mock.clone()),
        mock: Some(mock),
        rescan_interval: Duration::from_millis(50),
        autostart: None,
        open_desktop: None,
        open_window: opened.map(|opened| -> deckpilot_agent::WindowOpener {
            Box::new(move |id: &str| {
                opened.lock().unwrap().push(id.to_owned());
                Ok(())
            })
        }),
        key_store: keys.map(|keys| -> std::sync::Arc<dyn deckpilot_keystore::KeyStore> {
            std::sync::Arc::new(keys)
        }),
        auto_lock_check: Duration::from_millis(50),
        cancel_grace: cancel_grace.unwrap_or(Duration::from_secs(10)),
        clock: clock.unwrap_or_else(deckpilot_agent::system_clock),
        notifier: notifier.map(|n| -> std::sync::Arc<dyn deckpilot_core::notify::Notifier> { n }),
        screen_lock: screen.map(|screen| -> std::sync::Arc<dyn ScreenLockMonitor> { screen }),
    };
    let agent = Agent::start(options, lock)?;
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(agent.run(async move {
        let _ = stopped.await;
    }));
    Ok((task, stop))
}

async fn setup() -> TestAgent {
    setup_with(Extras::default()).await
}

async fn setup_with(extras: Extras) -> TestAgent {
    let paths = temp_paths();
    let (work, key) = seed(&paths);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "TEST-0001");
    let (task, stop) = start_agent_with(paths.clone(), mock.clone(), extras).unwrap();
    unlock_agent(&paths, &key).await;
    let agent = TestAgent {
        paths,
        mock,
        device,
        work,
        key,
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
        SqliteStore::open(&self.paths.database, &Encryption::Key(self.key.clone())).unwrap()
    }

    /// The profile's layout is on the device (the "Locked" notice before the
    /// unlock uses a single key).
    fn frames_shown(&self) -> bool {
        self.mock
            .wait_for(&self.device, TIMEOUT, |frames, _| frames.len() > 1)
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
    // A client that has seen the shutdown lets go of its connection, as the
    // app does. On Windows a connection that is still open keeps the old
    // pipe instance, and with it the name, from being reused.
    drop(subscriber);
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
    // It starts locked, like after a reboot, until the app unlocks it.
    let (task, stop) = start_agent(paths.clone(), agent.mock.clone()).unwrap();
    agent.task = Some(task);
    agent.stop = Some(stop);
    let locked = agent.wait_for_status(|s| !s.devices.is_empty()).await;
    assert_eq!(locked.security, SecurityState::Locked);
    unlock_agent(&paths, &agent.key).await;
    let status = agent
        .wait_for_status(|s| s.security == SecurityState::Unlocked)
        .await;
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
            appearance: None,
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
    let (store, key) = create_database(&paths);
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
    unlock_agent(&paths, &key).await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starts_locked_and_unlocks_only_with_the_right_key() {
    let paths = temp_paths();
    let (work, key) = seed(&paths);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "TEST-0001");
    let (task, stop) = start_agent(paths.clone(), mock.clone()).unwrap();
    let mut client = AgentClient::connect(&paths, ClientKind::Desktop)
        .await
        .unwrap();
    let status = |response: Response| match response {
        Response::Status { status } => *status,
        other => panic!("unexpected {other:?}"),
    };

    // Locked: no profile, no configuration requests, devices say so.
    let locked = status(client.request(Request::GetStatus).await.unwrap());
    assert_eq!(locked.security, SecurityState::Locked);
    assert!(locked.profiles.is_empty() && locked.active_profile.is_none());
    for request in [
        Request::ListProfiles,
        Request::RunAction {
            action: work.actions[0].id,
        },
        Request::ReloadConfiguration,
        Request::GetDatabaseKey,
        Request::SetActiveProfile { profile: work.id },
    ] {
        let refused = client.request(request).await;
        assert!(
            matches!(refused, Err(ClientError::Agent(ref e)) if e.code == ErrorCode::Locked),
            "{refused:?}"
        );
    }
    assert!(mock.wait_for(&device, TIMEOUT, |frames, _| !frames.is_empty()));

    // A wrong key changes nothing.
    let wrong = DatabaseKey::random().unwrap();
    let rejected = client
        .request(Request::Unlock {
            key: KeyMaterial::new(wrong.to_hex()),
        })
        .await;
    assert!(matches!(rejected, Err(ClientError::Agent(e)) if e.code == ErrorCode::Unauthorized));
    let malformed = client
        .request(Request::Unlock {
            key: KeyMaterial::new(zeroize::Zeroizing::new("xyz".into())),
        })
        .await;
    assert!(matches!(malformed, Err(ClientError::Agent(e)) if e.code == ErrorCode::InvalidRequest));
    assert_eq!(
        status(client.request(Request::GetStatus).await.unwrap()).security,
        SecurityState::Locked
    );

    // The right key unlocks; the app can then fetch it instead of asking
    // for the password again.
    client
        .request(Request::Unlock {
            key: KeyMaterial::new(key.to_hex()),
        })
        .await
        .unwrap();
    let unlocked = status(client.request(Request::GetStatus).await.unwrap());
    assert_eq!(unlocked.security, SecurityState::Unlocked);
    assert_eq!(unlocked.active_profile.unwrap().name, "Work");
    let Response::DatabaseKey { key: fetched } =
        client.request(Request::GetDatabaseKey).await.unwrap()
    else {
        panic!("expected the key")
    };
    assert_eq!(fetched.expose(), key.to_hex().as_str());
    assert_eq!(format!("{fetched:?}"), "KeyMaterial(<redacted>)");

    // Locking forgets the key again.
    client.request(Request::Lock).await.unwrap();
    let relocked = status(client.request(Request::GetStatus).await.unwrap());
    assert_eq!(relocked.security, SecurityState::Locked);
    assert!(relocked.active_profile.is_none());
    assert!(client.request(Request::GetDatabaseKey).await.is_err());

    client.request(Request::Shutdown).await.unwrap();
    tokio::time::timeout(TIMEOUT, task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(stop);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reports_missing_and_legacy_databases_without_opening_them() {
    for legacy in [false, true] {
        let paths = temp_paths();
        if legacy {
            // A database written by DeckPilot 0.3, unencrypted.
            SqliteStore::open(&paths.database, &Encryption::None).unwrap();
        }
        let (task, stop) = start_agent(paths.clone(), MockBackend::new()).unwrap();
        let mut client = AgentClient::connect(&paths, ClientKind::Desktop)
            .await
            .unwrap();
        let Response::Status { status } = client.request(Request::GetStatus).await.unwrap() else {
            panic!("expected status")
        };
        let expected = if legacy {
            SecurityState::LegacyPlaintext
        } else {
            SecurityState::Uninitialized
        };
        assert_eq!(status.security, expected);
        assert!(client.request(Request::ListProfiles).await.is_err());
        client.request(Request::Shutdown).await.unwrap();
        tokio::time::timeout(TIMEOUT, task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        drop(stop);
        // The agent created no database.
        assert_eq!(paths.database.exists(), legacy);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_windows_from_the_app_instead_of_drawing_them() {
    let paths = temp_paths();
    let (work, key) = seed(&paths);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "TEST-0001");
    let opened = Opened::default();
    let (task, stop) = start_agent_with(
        paths.clone(),
        mock.clone(),
        Extras {
            opened: Some(opened.clone()),
            ..Extras::default()
        },
    )
    .unwrap();
    let mut client = AgentClient::connect(&paths, ClientKind::Other)
        .await
        .unwrap();
    let press = Request::SimulateInput {
        device: device.clone(),
        control: ControlId::Key { index: 0 },
        input: ControlInput::Press,
    };

    // Locked, no app running: a key press starts the app for the unlock
    // window only; repeated presses do not open it again and again.
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while client.request(press.clone()).await.is_err() {
        assert!(tokio::time::Instant::now() < deadline, "device not ready");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while opened.lock().unwrap().is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no unlock window requested"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    client.request(press.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(*opened.lock().unwrap(), ["security.unlock"]);
    // The report of the last workflow needs the configuration.
    assert!(matches!(
        client.request(Request::GetLastWorkflowReport).await,
        Err(ClientError::Agent(e)) if e.code == ErrorCode::Locked
    ));

    // Identifiers are validated; nothing else can be sent.
    let invalid = client
        .request(Request::RequestWindow {
            window: "../../etc/passwd".into(),
        })
        .await;
    assert!(matches!(invalid, Err(ClientError::Agent(e)) if e.code == ErrorCode::InvalidRequest));

    // With the app connected, the request becomes an event for it.
    let mut app = AgentClient::connect(&paths, ClientKind::Desktop)
        .await
        .unwrap();
    app.subscribe().await.unwrap();
    client
        .request(Request::RequestWindow {
            window: "workflows.execution-summary".into(),
        })
        .await
        .unwrap();
    let event = tokio::time::timeout(TIMEOUT, async {
        loop {
            if let AgentEvent::OpenWindow { window } = app.next_event().await.unwrap() {
                return window;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(event, "workflows.execution-summary");
    assert_eq!(opened.lock().unwrap().len(), 1, "no second app was started");

    // After unlocking, the last workflow report is available.
    client
        .request(Request::Unlock {
            key: KeyMaterial::new(key.to_hex()),
        })
        .await
        .unwrap();
    let Response::LastWorkflow { report } = client
        .request(Request::GetLastWorkflowReport)
        .await
        .unwrap()
    else {
        panic!("expected the last workflow")
    };
    assert_eq!(report, None);
    let _ = work;
    client.request(Request::Shutdown).await.unwrap();
    tokio::time::timeout(TIMEOUT, task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(stop);
}

// System-managed keys. An in-memory credential store stands in for the
// Keychain, Credential Manager or Secret Service; no real store is touched.

async fn agent_status(paths: &AgentPaths) -> Option<AgentStatus> {
    let mut client = AgentClient::connect(paths, ClientKind::Other).await.ok()?;
    match client.request(Request::GetStatus).await.ok()? {
        Response::Status { status } => Some(*status),
        _ => None,
    }
}

async fn wait_for(paths: &AgentPaths, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        if let Some(status) = agent_status(paths).await
            && predicate(&status)
        {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not reached in time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn request(paths: &AgentPaths, request: Request) -> Result<Response, ClientError> {
    AgentClient::connect(paths, ClientKind::Desktop)
        .await
        .unwrap()
        .request(request)
        .await
}

async fn stop_agent(paths: &AgentPaths, (task, stop): Running) {
    request(paths, Request::Shutdown).await.unwrap();
    tokio::time::timeout(TIMEOUT, task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(stop);
}

async fn agent_key(paths: &AgentPaths) -> DatabaseKey {
    match request(paths, Request::GetDatabaseKey).await.unwrap() {
        Response::DatabaseKey { key } => DatabaseKey::from_hex(key.expose()).unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

fn with_keys(keys: &MemoryKeyStore) -> Extras {
    Extras {
        keys: Some(keys.clone()),
        ..Extras::default()
    }
}

fn unlocked(status: &AgentStatus) -> bool {
    status.security == SecurityState::Unlocked
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fresh_install_opens_without_a_password_and_after_restarts() {
    let paths = temp_paths();
    let keys = MemoryKeyStore::new();
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    let status = wait_for(&paths, unlocked).await;
    assert_eq!(status.protection, Some(ProtectionMode::SystemManaged));
    assert_eq!(status.lock_reason, None);
    assert_eq!(keys.ids().len(), 1);

    // Configure something through the key the app would receive.
    let key = agent_key(&paths).await;
    let profile = {
        let store = SqliteStore::open(&paths.database, &Encryption::Key(key.clone())).unwrap();
        let profile = Profile::new("Home");
        store.save_profile(&profile).unwrap();
        store.set_active_profile(Some(profile.id)).unwrap();
        profile
    };
    stop_agent(&paths, running).await;

    // A restart (login): unlocked again, the active profile is restored,
    // and no new key was created.
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    let status = wait_for(&paths, unlocked).await;
    assert_eq!(status.active_profile.map(|p| p.id), Some(profile.id));
    assert!(agent_key(&paths).await.matches(&key));
    assert_eq!(keys.ids().len(), 1);
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unavailable_credential_store_is_a_clear_locked_state() {
    let paths = temp_paths();
    let keys = MemoryKeyStore::new();
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    wait_for(&paths, unlocked).await;
    stop_agent(&paths, running).await;

    for missing in [false, true] {
        if missing {
            // The entry was deleted, e.g. the keychain was reset.
            keys.set_unavailable(false);
            assert_eq!(keys.ids().len(), 1);
            keys.clear();
        } else {
            keys.set_unavailable(true);
        }
        let running =
            start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
        let status = wait_for(&paths, |s| {
            !s.waiting_for_key_store && s.security_problem.is_some()
        })
        .await;
        assert_eq!(status.security, SecurityState::Locked);
        assert_eq!(status.lock_reason, Some(LockReason::KeyUnavailable));
        assert_eq!(status.protection, Some(ProtectionMode::SystemManaged));
        assert!(matches!(
            request(&paths, Request::ListProfiles).await,
            Err(ClientError::Agent(e)) if e.code == ErrorCode::Locked
        ));
        // Retrying while the store is still unavailable fails again, without
        // creating a new key or a new database.
        assert!(request(&paths, Request::SystemUnlock).await.is_err());
        assert!(paths.database.exists());
        stop_agent(&paths, running).await;
    }
    assert_eq!(keys.ids(), Vec::new(), "no replacement key was created");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retrying_after_the_store_becomes_available_unlocks() {
    let paths = temp_paths();
    let keys = MemoryKeyStore::new();
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    wait_for(&paths, unlocked).await;
    stop_agent(&paths, running).await;

    keys.set_unavailable(true);
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    wait_for(&paths, |s| {
        s.lock_reason == Some(LockReason::KeyUnavailable)
    })
    .await;
    keys.set_unavailable(false);
    request(&paths, Request::SystemUnlock).await.unwrap();
    let status = wait_for(&paths, unlocked).await;
    assert_eq!(status.security_problem, None);
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_credential_store_on_first_run_creates_nothing() {
    let paths = temp_paths();
    let keys = MemoryKeyStore::new();
    keys.set_unavailable(true);
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    let status = wait_for(&paths, |s| s.security_problem.is_some()).await;
    assert_eq!(status.security, SecurityState::Uninitialized);
    assert!(!paths.database.exists());
    assert!(
        !paths
            .database
            .with_file_name("deckpilot.vault.json")
            .exists()
    );
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_password_database_switches_to_a_system_key() {
    let paths = temp_paths();
    let (profile, key) = seed(&paths);
    let keys = MemoryKeyStore::new();
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    // Password protection: the agent waits for the app.
    let status = wait_for(&paths, |s| s.security == SecurityState::Locked).await;
    assert_eq!(status.protection, Some(ProtectionMode::Password));
    assert_eq!(status.lock_reason, Some(LockReason::Startup));
    assert_eq!(keys.ids(), Vec::new());
    unlock_agent(&paths, &key).await;

    // The current key is the proof of access.
    let wrong = deckpilot_storage::crypto::SecretKey::random().unwrap();
    assert!(matches!(
        request(&paths, Request::UseSystemKey { key: KeyMaterial::new(wrong.to_hex()) }).await,
        Err(ClientError::Agent(e)) if e.code == ErrorCode::Unauthorized
    ));
    request(
        &paths,
        Request::UseSystemKey {
            key: KeyMaterial::new(key.to_hex()),
        },
    )
    .await
    .unwrap();
    let status = wait_for(&paths, unlocked).await;
    assert_eq!(status.protection, Some(ProtectionMode::SystemManaged));
    assert_eq!(status.active_profile.map(|p| p.id), Some(profile.id));
    let new_key = agent_key(&paths).await;
    assert!(!new_key.matches(&key));
    assert_eq!(keys.ids().len(), 1);
    stop_agent(&paths, running).await;

    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    let status = wait_for(&paths, unlocked).await;
    assert_eq!(status.active_profile.map(|p| p.id), Some(profile.id));
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_switch_leaves_the_password_protection() {
    let paths = temp_paths();
    let (_, key) = seed(&paths);
    let keys = MemoryKeyStore::new();
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    wait_for(&paths, |s| s.security == SecurityState::Locked).await;
    unlock_agent(&paths, &key).await;
    keys.set_unavailable(true);
    assert!(
        request(
            &paths,
            Request::UseSystemKey {
                key: KeyMaterial::new(key.to_hex()),
            },
        )
        .await
        .is_err()
    );
    // Still open with the old key and still protected by the password.
    let status = wait_for(&paths, unlocked).await;
    assert_eq!(status.protection, Some(ProtectionMode::Password));
    assert!(agent_key(&paths).await.matches(&key));
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locks_after_inactivity_and_sleep_when_configured() {
    use deckpilot_storage::policy::{AutoLock, SecurityPolicy, set_security_policy};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    let paths = temp_paths();
    let keys = MemoryKeyStore::new();
    let offset = Arc::new(AtomicU64::new(0));
    let clock: Clock = {
        let offset = offset.clone();
        Arc::new(move || {
            std::time::SystemTime::now() + Duration::from_secs(offset.load(Ordering::SeqCst))
        })
    };
    let extras = || Extras {
        keys: Some(keys.clone()),
        clock: Some(clock.clone()),
        ..Extras::default()
    };
    let running = start_agent_with(paths.clone(), MockBackend::new(), extras()).unwrap();
    wait_for(&paths, unlocked).await;
    let key = agent_key(&paths).await;
    let store = SqliteStore::open(&paths.database, &Encryption::Key(key)).unwrap();

    // Default: never locks, however long it is idle.
    offset.fetch_add(3 * 60 * 60, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(unlocked(&agent_status(&paths).await.unwrap()));

    // After 15 minutes without activity.
    let policy = |auto_lock| SecurityPolicy {
        auto_lock,
        ..SecurityPolicy::default()
    };
    set_security_policy(&store, &policy(AutoLock::Idle15Minutes)).unwrap();
    request(&paths, Request::ReloadConfiguration).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(unlocked(&agent_status(&paths).await.unwrap()));
    offset.fetch_add(16 * 60, Ordering::SeqCst);
    let status = wait_for(&paths, |s| s.security == SecurityState::Locked).await;
    assert_eq!(status.lock_reason, Some(LockReason::Idle));

    // Unlocking again with the system key (after the app checked presence).
    request(&paths, Request::SystemUnlock).await.unwrap();
    wait_for(&paths, unlocked).await;

    // When the computer sleeps: a jump of the wall clock between checks.
    set_security_policy(&store, &policy(AutoLock::Sleep)).unwrap();
    request(&paths, Request::ReloadConfiguration).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(unlocked(&agent_status(&paths).await.unwrap()));
    offset.fetch_add(10 * 60, Ordering::SeqCst);
    let status = wait_for(&paths, |s| s.security == SecurityState::Locked).await;
    assert_eq!(status.lock_reason, Some(LockReason::Sleep));
    drop(store);
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_database_is_encrypted_with_a_system_key() {
    let paths = temp_paths();
    {
        let store = SqliteStore::open(&paths.database, &Encryption::None).unwrap();
        store.save_profile(&Profile::new("Old")).unwrap();
    }
    let keys = MemoryKeyStore::new();
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    let status = wait_for(&paths, unlocked).await;
    assert_eq!(status.protection, Some(ProtectionMode::SystemManaged));
    assert_eq!(status.profiles.len(), 1);
    // The original is kept until the user removes it.
    assert!(
        paths
            .database
            .with_file_name("deckpilot.unencrypted-backup.db")
            .exists()
    );
    stop_agent(&paths, running).await;
}

/// A system-key configuration with an active profile, project and layout,
/// as the user left it.
fn system_configuration(paths: &AgentPaths, keys: &MemoryKeyStore) -> (Profile, ProjectKey) {
    let key = Vault::new(&paths.database).initialize_system(keys).unwrap();
    let store = SqliteStore::open(&paths.database, &Encryption::Key(key)).unwrap();
    let mut profile = Profile::new("Work");
    for key in ["SHOP", "CMR"] {
        profile
            .add_project(Project::new(ProjectKey::new(key).unwrap(), key))
            .unwrap();
    }
    let layout = DeviceLayout::generate("Main", &models::stream_deck_mk2(), &profile);
    let layout_id = layout.id;
    profile.layouts.push(layout);
    profile.activate_layout(layout_id).unwrap();
    store.save_profile(&profile).unwrap();
    store.set_active_profile(Some(profile.id)).unwrap();
    let cmr = profile.projects[1].id;
    store.set_active_project(profile.id, Some(cmr)).unwrap();
    (profile, ProjectKey::new("CMR").unwrap())
}

fn restored(status: &AgentStatus, profile: &Profile, project: &ProjectKey) -> bool {
    status.security == SecurityState::Unlocked
        && status.active_profile.as_ref().map(|p| p.id) == Some(profile.id)
        && status.active_project.as_ref().map(|p| &p.key) == Some(project)
        && status.devices.iter().any(|d| {
            matches!(&d.layout, LayoutStatus::Compatible { layout, .. } if *layout == profile.layouts[0].id)
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unlocking_restores_profile_project_and_layouts() {
    let paths = temp_paths();
    let keys = MemoryKeyStore::new();
    let (profile, project) = system_configuration(&paths, &keys);
    let mock = MockBackend::new();
    mock.plug(&models::stream_deck_mk2(), "TEST-0001");
    // Login: the agent opens the configuration by itself.
    let running = start_agent_with(paths.clone(), mock.clone(), with_keys(&keys)).unwrap();
    wait_for(&paths, |s| restored(s, &profile, &project)).await;
    // After a lock and an unlock, everything is back as it was.
    request(&paths, Request::Lock).await.unwrap();
    let locked = wait_for(&paths, |s| s.security == SecurityState::Locked).await;
    assert_eq!(locked.active_profile, None);
    request(&paths, Request::SystemUnlock).await.unwrap();
    wait_for(&paths, |s| restored(s, &profile, &project)).await;
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locks_when_the_screen_locks_where_it_can_be_detected() {
    use deckpilot_storage::policy::{AutoLock, SecurityPolicy, set_security_policy};

    let paths = temp_paths();
    let keys = MemoryKeyStore::new();
    let screen = std::sync::Arc::new(ScriptedScreenLock::default());
    let extras = || Extras {
        keys: Some(keys.clone()),
        screen: Some(screen.clone()),
        ..Extras::default()
    };
    let running = start_agent_with(paths.clone(), MockBackend::new(), extras()).unwrap();
    let status = wait_for(&paths, unlocked).await;
    assert!(status.screen_lock_detection);
    let store =
        SqliteStore::open(&paths.database, &Encryption::Key(agent_key(&paths).await)).unwrap();

    // Not configured: the screen lock is ignored.
    screen.set_locked(true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(unlocked(&agent_status(&paths).await.unwrap()));
    screen.set_locked(false);

    set_security_policy(
        &store,
        &SecurityPolicy {
            auto_lock: AutoLock::ScreenLock,
            ..SecurityPolicy::default()
        },
    )
    .unwrap();
    request(&paths, Request::ReloadConfiguration).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    screen.set_locked(true);
    let status = wait_for(&paths, |s| s.security == SecurityState::Locked).await;
    assert_eq!(status.lock_reason, Some(LockReason::ScreenLocked));

    // Unlocking while the screen is still locked does not lock again; the
    // next screen lock does.
    request(&paths, Request::SystemUnlock).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(unlocked(&agent_status(&paths).await.unwrap()));
    screen.set_locked(false);
    tokio::time::sleep(Duration::from_millis(200)).await;
    screen.set_locked(true);
    wait_for(&paths, |s| s.lock_reason == Some(LockReason::ScreenLocked)).await;
    drop(store);
    stop_agent(&paths, running).await;

    // Without a monitor, detection is reported as unavailable.
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    assert!(!wait_for(&paths, unlocked).await.screen_lock_detection);
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_store_is_retried_but_a_refusal_is_not() {
    let paths = temp_paths();
    let keys = MemoryKeyStore::new();
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    wait_for(&paths, unlocked).await;
    stop_agent(&paths, running).await;

    // The keyring daemon is not up yet at login: retried by itself.
    keys.set_unreachable(true);
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    wait_for(&paths, |s| {
        s.lock_reason == Some(LockReason::KeyUnavailable)
    })
    .await;
    keys.set_unreachable(false);
    let status = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = agent_status(&paths).await
                && unlocked(&status)
            {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("unlocked after the store became reachable");
    assert_eq!(status.security_problem, None);
    stop_agent(&paths, running).await;

    // Refused (locked keychain, cancelled prompt): asked once, not again.
    keys.set_unavailable(true);
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    wait_for(&paths, |s| {
        s.lock_reason == Some(LockReason::KeyUnavailable)
    })
    .await;
    let attempts = keys.attempts();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(keys.attempts(), attempts, "a refusal is not retried");
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_database_is_reported_not_replaced() {
    let paths = temp_paths();
    let keys = MemoryKeyStore::new();
    system_configuration(&paths, &keys);
    std::fs::remove_file(&paths.database).unwrap();
    let running = start_agent_with(paths.clone(), MockBackend::new(), with_keys(&keys)).unwrap();
    let status = wait_for(&paths, |s| s.security_problem.is_some()).await;
    assert_eq!(status.security, SecurityState::Uninitialized);
    assert!(status.security_problem.unwrap().contains("missing"));
    assert!(request(&paths, Request::SystemUnlock).await.is_err());
    assert!(!paths.database.exists());
    assert_eq!(keys.ids().len(), 1);
    stop_agent(&paths, running).await;
}

/// A graph workflow that waits `ms` and then announces the project.
fn waiting_workflow(profile: &Profile, ms: u32) -> deckpilot_core::Workflow {
    let mut workflow = deckpilot_core::Workflow::new("Wait and announce");
    workflow.graph = Some(
        serde_json::from_value(json!({
            "version": 1,
            "nodes": [
                {"id": "start", "type": "trigger"},
                {"id": "wait", "type": "delay", "ms": ms},
                {"id": "announce", "type": "action", "action": profile.actions[1].id},
            ],
            "edges": [
                {"from": "start", "to": "wait"},
                {"from": "wait", "to": "announce"},
            ],
        }))
        .unwrap(),
    );
    workflow
}

/// Adds `workflow` to the agent's profile and reloads it.
async fn add_workflow(agent: &TestAgent, workflow: deckpilot_core::Workflow) {
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    profile.workflows.push(workflow);
    agent.store().save_profile(&profile).unwrap();
    agent
        .client()
        .await
        .request(Request::ReloadConfiguration)
        .await
        .unwrap();
}

/// Events of one execution until it finished.
async fn execution_events(
    client: &mut AgentClient,
    id: deckpilot_core::ExecutionId,
) -> (Vec<AgentEvent>, deckpilot_core::ExecutionSummary) {
    let mut events = Vec::new();
    loop {
        let event = tokio::time::timeout(TIMEOUT, client.next_event())
            .await
            .expect("execution events in time")
            .unwrap();
        match event {
            AgentEvent::ExecutionFinished { execution } if execution.id == id => {
                return (events, execution);
            }
            AgentEvent::ExecutionProgress { execution, .. } if execution == id => {
                events.push(event);
            }
            _ => {}
        }
    }
}

async fn start_workflow(
    client: &mut AgentClient,
    workflow: deckpilot_core::WorkflowId,
) -> deckpilot_core::ExecutionSummary {
    match client
        .request(Request::StartWorkflow { workflow })
        .await
        .unwrap()
    {
        Response::ExecutionStarted { execution } => execution,
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runs_workflows_in_the_background_with_progress_and_history() {
    let agent = setup().await;
    let workflow = deckpilot_core::Workflow::new("Switch and announce")
        .step(agent.work.actions[0].id, deckpilot_core::OnFailure::Stop)
        .step(agent.work.actions[1].id, deckpilot_core::OnFailure::Stop);
    let workflow_id = workflow.id;
    add_workflow(&agent, workflow).await;

    let mut events = agent.client().await;
    events.subscribe().await.unwrap();
    let mut client = agent.client().await;
    let started = start_workflow(&mut client, workflow_id).await;
    assert_eq!(started.state, deckpilot_core::ExecutionState::Running);
    assert_eq!(started.total_nodes, 2);
    assert_eq!(started.trigger, deckpilot_core::ExecutionTrigger::App);

    let (progress, finished) = execution_events(&mut events, started.id).await;
    // Started and finished per step, with real counts.
    assert_eq!(progress.len(), 4);
    let Some(AgentEvent::ExecutionProgress {
        completed, total, ..
    }) = progress.last()
    else {
        panic!("expected progress")
    };
    assert_eq!((*completed, *total), (2, 2));
    assert_eq!(finished.state, deckpilot_core::ExecutionState::Succeeded);
    assert_eq!(finished.completed_nodes, 2);
    assert!(finished.finished_at.is_some());
    // The workflow's project switch reached the agent.
    agent
        .wait_for_status(|s| active_key(s) == Some("CMR"))
        .await;

    let Response::Execution { execution } = client
        .request(Request::GetExecution {
            execution: started.id,
        })
        .await
        .unwrap()
    else {
        panic!("expected an execution")
    };
    let nodes: Vec<_> = execution.nodes.iter().map(|n| n.node.to_string()).collect();
    assert_eq!(nodes, ["step_1", "step_2"]);
    assert_eq!(execution.nodes[1].message, "CMR");

    // Recorded in the encrypted database.
    let stored = agent.store().execution(started.id).unwrap().unwrap();
    assert_eq!(
        stored.summary.state,
        deckpilot_core::ExecutionState::Succeeded
    );
    let Response::Executions { executions } = client
        .request(Request::ListExecutions {
            workflow: Some(workflow_id),
            limit: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected executions")
    };
    assert_eq!(executions.len(), 1);

    client
        .request(Request::ClearExecutionHistory { workflow: None })
        .await
        .unwrap();
    assert!(agent.store().execution(started.id).unwrap().is_none());
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_workflows_can_be_cancelled_and_do_not_block_the_agent() {
    let agent = setup().await;
    let workflow = waiting_workflow(&agent.work, 60_000);
    let workflow_id = workflow.id;
    add_workflow(&agent, workflow).await;

    let mut events = agent.client().await;
    events.subscribe().await.unwrap();
    let mut client = agent.client().await;
    let started = start_workflow(&mut client, workflow_id).await;

    // The agent keeps answering while the workflow waits.
    let begun = std::time::Instant::now();
    agent.status().await;
    assert!(begun.elapsed() < Duration::from_secs(5));
    // A workflow runs once at a time.
    let again = client
        .request(Request::StartWorkflow {
            workflow: workflow_id,
        })
        .await;
    assert!(matches!(again, Err(ClientError::Agent(e)) if e.message.contains("already running")));
    let Response::Executions { executions } = client
        .request(Request::ListExecutions {
            workflow: None,
            limit: Some(10),
        })
        .await
        .unwrap()
    else {
        panic!("expected executions")
    };
    assert_eq!(executions[0].state, deckpilot_core::ExecutionState::Running);

    client
        .request(Request::CancelExecution {
            execution: started.id,
        })
        .await
        .unwrap();
    let (_, finished) = execution_events(&mut events, started.id).await;
    assert_eq!(finished.state, deckpilot_core::ExecutionState::Cancelled);
    let record = agent.store().execution(started.id).unwrap().unwrap();
    assert!(
        record.nodes.iter().all(|n| n.node.as_str() == "start"
            || n.status == deckpilot_core::graph::NodeStatus::Cancelled),
        "{:?}",
        record.nodes
    );
    let unknown = client
        .request(Request::CancelExecution {
            execution: started.id,
        })
        .await;
    assert!(matches!(unknown, Err(ClientError::Agent(e)) if e.code == ErrorCode::NotFound));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_bound_workflows_run_in_the_background() {
    let agent = setup().await;
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let workflow = waiting_workflow(&profile, 10);
    let workflow_id = workflow.id;
    profile.workflows.push(workflow);
    profile.layouts[0].pages[0]
        .bindings
        .push(deckpilot_core::ControlBinding {
            control: ControlId::Key { index: 5 },
            trigger: deckpilot_core::Trigger::Press,
            target: deckpilot_core::BindingTarget::Workflow {
                workflow: workflow_id,
            },
            label: None,
            appearance: None,
        });
    agent.store().save_profile(&profile).unwrap();
    let mut events = agent.client().await;
    events.subscribe().await.unwrap();
    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();
    client
        .request(Request::SimulateInput {
            device: agent.device.clone(),
            control: ControlId::Key { index: 5 },
            input: ControlInput::Press,
        })
        .await
        .unwrap();
    let started = loop {
        if let AgentEvent::ExecutionStarted { execution } =
            tokio::time::timeout(TIMEOUT, events.next_event())
                .await
                .unwrap()
                .unwrap()
        {
            break execution;
        }
    };
    assert_eq!(started.workflow, workflow_id);
    assert_eq!(started.trigger, deckpilot_core::ExecutionTrigger::Device);
    let (_, finished) = execution_events(&mut events, started.id).await;
    assert_eq!(finished.state, deckpilot_core::ExecutionState::Succeeded);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runs_of_a_stopped_agent_are_recorded_as_interrupted() {
    let agent = setup().await;
    let workflow = waiting_workflow(&agent.work, 60_000);
    let workflow_id = workflow.id;
    add_workflow(&agent, workflow).await;
    let mut client = agent.client().await;
    let started = start_workflow(&mut client, workflow_id).await;
    assert_eq!(
        agent
            .store()
            .execution(started.id)
            .unwrap()
            .unwrap()
            .summary
            .state,
        deckpilot_core::ExecutionState::Running
    );

    // A record left running by an agent that crashed is marked when the
    // database is opened.
    let profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let crashed = deckpilot_core::ExecutionRecord::start(
        deckpilot_core::ExecutionId::new(),
        profile.workflow(workflow_id).unwrap(),
        &profile,
        deckpilot_core::ExecutionTrigger::Device,
        1,
    );
    agent.store().save_execution(&crashed).unwrap();

    let (paths, key, mock) = (agent.paths.clone(), agent.key.clone(), agent.mock.clone());
    // Clients close their connections before the agent is restarted (see
    // `single_instance_shutdown_and_client_reconnect`).
    drop(client);
    // Stopping the agent interrupts the run that is still waiting.
    agent.shutdown().await;
    let record = SqliteStore::open(&paths.database, &Encryption::Key(key.clone()))
        .unwrap()
        .execution(started.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        record.summary.state,
        deckpilot_core::ExecutionState::Interrupted
    );

    let running = start_agent(paths.clone(), mock).unwrap();
    unlock_agent(&paths, &key).await;
    wait_for(&paths, unlocked).await;
    let record = SqliteStore::open(&paths.database, &Encryption::Key(key))
        .unwrap()
        .execution(crashed.summary.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        record.summary.state,
        deckpilot_core::ExecutionState::Interrupted
    );
    stop_agent(&paths, running).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn older_clients_do_not_receive_execution_events() {
    let agent = setup().await;
    let workflow = waiting_workflow(&agent.work, 10);
    let workflow_id = workflow.id;
    add_workflow(&agent, workflow).await;
    let mut old = AgentClient::connect_with_protocol(&agent.paths, ClientKind::Other, 4)
        .await
        .unwrap();
    old.subscribe().await.unwrap();
    let mut client = agent.client().await;
    // `runWorkflow` still answers with the report when the run ends.
    let Response::WorkflowReport { report } = client
        .request(Request::RunWorkflow {
            workflow: workflow_id,
        })
        .await
        .unwrap()
    else {
        panic!("expected a report")
    };
    assert_eq!(report.status, ActionStatus::Succeeded);
    // Only events of protocol 4 arrived.
    while let Ok(Ok(event)) =
        tokio::time::timeout(Duration::from_millis(300), old.next_event()).await
    {
        assert!(event.since_protocol() <= 4, "{event:?}");
    }
    agent.shutdown().await;
}

/// A graph workflow: select `project`, wait `ms`, then announce the
/// project of the run's context.
fn select_wait_announce(
    profile: &mut Profile,
    project: &str,
    ms: u32,
    scope: deckpilot_core::ProjectScope,
) -> deckpilot_core::Workflow {
    let project_id = profile
        .projects
        .iter()
        .find(|p| p.key.as_str() == project)
        .unwrap()
        .id;
    let select = registry()
        .create_instance(
            &format!("Select {project}"),
            actions::select_id(),
            &ActionParameters::new().with(
                actions::PROJECT_ID,
                deckpilot_core::ParameterValue::Project(project_id),
            ),
        )
        .unwrap();
    let select_id = select.id;
    profile.actions.push(select);
    let mut workflow = deckpilot_core::Workflow::new(format!("Work on {project}"));
    workflow.project_scope = Some(scope);
    workflow.graph = Some(
        serde_json::from_value(json!({
            "version": 1,
            "nodes": [
                {"id": "start", "type": "trigger"},
                {"id": "select", "type": "action", "action": select_id},
                {"id": "wait", "type": "delay", "ms": ms},
                {"id": "announce", "type": "action", "action": profile.actions[1].id},
            ],
            "edges": [
                {"from": "start", "to": "select"},
                {"from": "select", "to": "wait"},
                {"from": "wait", "to": "announce"},
            ],
        }))
        .unwrap(),
    );
    workflow
}

async fn execution(
    client: &mut AgentClient,
    id: deckpilot_core::ExecutionId,
) -> deckpilot_core::ExecutionRecord {
    match client
        .request(Request::GetExecution { execution: id })
        .await
        .unwrap()
    {
        Response::Execution { execution } => *execution,
        other => panic!("unexpected {other:?}"),
    }
}

fn announced(record: &deckpilot_core::ExecutionRecord) -> String {
    record
        .nodes
        .iter()
        .find(|n| n.node.as_str() == "announce")
        .map(|n| n.message.clone())
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_runs_keep_their_own_project_context() {
    let agent = setup().await;
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let cmr = select_wait_announce(
        &mut profile,
        "CMR",
        400,
        deckpilot_core::ProjectScope::Execution,
    );
    let cms = select_wait_announce(
        &mut profile,
        "CMS",
        100,
        deckpilot_core::ProjectScope::Execution,
    );
    let (slow, fast) = (cmr.id, cms.id);
    profile.workflows.extend([cmr, cms]);
    agent.store().save_profile(&profile).unwrap();
    let mut events = agent.client().await;
    events.subscribe().await.unwrap();
    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();

    // Both run at the same time, each selecting another project.
    let first = start_workflow(&mut client, slow).await;
    let second = start_workflow(&mut client, fast).await;
    assert_eq!(first.initial_project, second.initial_project);
    let mut finished = Vec::new();
    while finished.len() < 2 {
        if let AgentEvent::ExecutionFinished { execution } =
            tokio::time::timeout(TIMEOUT, events.next_event())
                .await
                .unwrap()
                .unwrap()
        {
            finished.push(execution.id);
        }
    }
    let first = execution(&mut client, first.id).await;
    let second = execution(&mut client, second.id).await;
    assert_eq!(announced(&first), "CMR");
    assert_eq!(announced(&second), "CMS");
    assert!(matches!(
        first.summary.project,
        Some(deckpilot_core::ProjectOutcome::KeptInRun { .. })
    ));
    // The application's active project never changed.
    assert_eq!(active_key(&agent.status().await), Some("SHOP"));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_permitted_runs_change_the_active_project_and_never_over_a_newer_choice() {
    let agent = setup().await;
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let workflow = select_wait_announce(
        &mut profile,
        "CMR",
        300,
        deckpilot_core::ProjectScope::Application,
    );
    let workflow_id = workflow.id;
    profile.workflows.push(workflow);
    agent.store().save_profile(&profile).unwrap();
    let mut events = agent.client().await;
    events.subscribe().await.unwrap();
    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();

    // Nothing else changes the active project: the run's project is applied.
    let run = start_workflow(&mut client, workflow_id).await;
    assert_eq!(
        run.project_scope,
        Some(deckpilot_core::ProjectScope::Application)
    );
    let (_, done) = execution_events(&mut events, run.id).await;
    assert!(matches!(
        done.project,
        Some(deckpilot_core::ProjectOutcome::Applied { .. })
    ));
    agent
        .wait_for_status(|s| active_key(s) == Some("CMR"))
        .await;

    // The user picks another project while the run waits: their choice
    // stands and the run records why it did not switch.
    let cms = profile
        .projects
        .iter()
        .find(|p| p.key.as_str() == "CMS")
        .unwrap()
        .id;
    let shop = profile.projects[0].id;
    client
        .request(Request::SelectProject { project: shop })
        .await
        .unwrap();
    let run = start_workflow(&mut client, workflow_id).await;
    client
        .request(Request::SelectProject { project: cms })
        .await
        .unwrap();
    let (_, done) = execution_events(&mut events, run.id).await;
    match done.project {
        Some(deckpilot_core::ProjectOutcome::NotApplied { reason, .. }) => {
            assert!(reason.contains("changed during the run"), "{reason}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(active_key(&agent.status().await), Some("CMS"));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_interrupts_a_running_action_at_once() {
    let agent = setup().await;
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let wait = registry()
        .create_instance(
            "Wait a while",
            general::wait_id(),
            &ActionParameters::new().with("seconds", deckpilot_core::ParameterValue::Integer(300)),
        )
        .unwrap();
    let wait_id = wait.id;
    profile.actions.push(wait);
    let mut workflow = deckpilot_core::Workflow::new("Long wait");
    workflow.graph = Some(
        serde_json::from_value(json!({
            "version": 1,
            "nodes": [
                {"id": "start", "type": "trigger"},
                {"id": "wait", "type": "action", "action": wait_id},
            ],
            "edges": [{"from": "start", "to": "wait"}],
        }))
        .unwrap(),
    );
    let workflow_id = workflow.id;
    profile.workflows.push(workflow);
    agent.store().save_profile(&profile).unwrap();
    let mut events = agent.client().await;
    events.subscribe().await.unwrap();
    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();

    let run = start_workflow(&mut client, workflow_id).await;
    // Wait until the action runs and reports progress.
    loop {
        if let AgentEvent::ExecutionProgress {
            event: deckpilot_core::graph::ExecutionEvent::NodeProgress { .. },
            ..
        } = tokio::time::timeout(TIMEOUT, events.next_event())
            .await
            .unwrap()
            .unwrap()
        {
            break;
        }
    }
    let begun = std::time::Instant::now();
    client
        .request(Request::CancelExecution { execution: run.id })
        .await
        .unwrap();
    let (_, finished) = execution_events(&mut events, run.id).await;
    assert!(
        begun.elapsed() < Duration::from_secs(3),
        "{:?}",
        begun.elapsed()
    );
    assert_eq!(finished.state, deckpilot_core::ExecutionState::Cancelled);
    let record = execution(&mut client, run.id).await;
    let node = record
        .nodes
        .iter()
        .find(|n| n.node.as_str() == "wait")
        .unwrap();
    assert_eq!(node.status, deckpilot_core::graph::NodeStatus::Cancelled);
    assert_eq!(node.error.as_ref().unwrap().code, "cancelled");
    agent.shutdown().await;
}

/// A workflow running `system.run-command` with the platform's shell
/// bound as the `tools.shell` resource.
fn command_workflow(agent: &TestAgent, script: &str) -> deckpilot_core::WorkflowId {
    let mut machine = agent
        .store()
        .load_machine_config()
        .unwrap()
        .unwrap_or_else(|| deckpilot_core::MachineConfig::new(Os::current()));
    machine.set_binding(deckpilot_core::MachineResourceBinding {
        key: deckpilot_core::ResourceKey::new("tools.shell").unwrap(),
        kind: deckpilot_core::ResourceKind::Other,
        value: if cfg!(windows) { "cmd" } else { "sh" }.into(),
    });
    agent.store().save_machine_config(&machine).unwrap();
    let arguments = if cfg!(windows) {
        format!("/C\n{script}")
    } else {
        format!("-c\n{script}")
    };
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let action = registry()
        .create_instance(
            "Print",
            deckpilot_modules::system::actions::id(deckpilot_modules::system::actions::RUN_COMMAND),
            &ActionParameters::new()
                .with(
                    "program",
                    deckpilot_core::ParameterValue::Resource(deckpilot_core::ResourceRef::Fixed {
                        key: deckpilot_core::ResourceKey::new("tools.shell").unwrap(),
                    }),
                )
                .with("arguments", deckpilot_core::ParameterValue::Text(arguments)),
        )
        .unwrap();
    let mut workflow = deckpilot_core::Workflow::new("Run a command");
    workflow.graph = Some(
        serde_json::from_value(json!({
            "version": 1,
            "nodes": [
                {"id": "start", "type": "trigger"},
                {"id": "cmd", "type": "action", "action": action.id},
                {"id": "check", "type": "condition", "predicate": {
                    "type": "equals",
                    "reference": {"node": "cmd", "output": "exitCode"},
                    "value": "0",
                }},
            ],
            "edges": [
                {"from": "start", "to": "cmd"},
                {"from": "cmd", "to": "check"},
            ],
        }))
        .unwrap(),
    );
    let id = workflow.id;
    profile.actions.push(action);
    profile.workflows.push(workflow);
    agent.store().save_profile(&profile).unwrap();
    id
}

fn set_system(agent: &TestAgent, enabled: bool, allow_commands: bool) {
    agent
        .store()
        .set_module_state(
            &deckpilot_modules::system::module_id(),
            &ModuleSettings {
                enabled,
                config: ActionParameters::new().with(
                    deckpilot_modules::system::ALLOW_COMMANDS,
                    deckpilot_core::ParameterValue::Boolean(allow_commands),
                ),
            },
        )
        .unwrap();
}

fn node<'a>(
    record: &'a deckpilot_core::ExecutionRecord,
    id: &str,
) -> &'a deckpilot_core::graph::NodeReport {
    record.nodes.iter().find(|n| n.node.as_str() == id).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn system_commands_are_allowed_per_computer_and_their_output_is_not_kept() {
    let agent = setup().await;
    let workflow = command_workflow(&agent, "echo s3cret-output");
    let mut client = agent.client().await;
    client.subscribe().await.unwrap();
    client.request(Request::ReloadConfiguration).await.unwrap();

    // Off until allowed on this computer.
    let started = start_workflow(&mut client, workflow).await;
    let (_, finished) = execution_events(&mut client, started.id).await;
    assert_eq!(finished.state, deckpilot_core::ExecutionState::Failed);
    let record = agent.store().execution(started.id).unwrap().unwrap();
    let error = node(&record, "cmd").error.clone().unwrap();
    assert_eq!(error.action_code.as_deref(), Some("commands-disabled"));

    set_system(&agent, true, true);
    client.request(Request::ReloadConfiguration).await.unwrap();
    let started = start_workflow(&mut client, workflow).await;
    let (events, finished) = execution_events(&mut client, started.id).await;
    assert_eq!(
        finished.state,
        deckpilot_core::ExecutionState::Succeeded,
        "{}",
        finished.message
    );
    let record = agent.store().execution(started.id).unwrap().unwrap();
    let cmd = node(&record, "cmd");
    // Later nodes saw the output (the condition passed on the exit code);
    // history and live events never contain the program's output.
    assert_eq!(cmd.output["exitCode"], "0");
    assert!(!cmd.output.contains_key("stdout"));
    let stored = serde_json::to_string(&record).unwrap();
    let live = serde_json::to_string(&events).unwrap();
    assert!(!stored.contains("s3cret-output") && !live.contains("s3cret-output"));

    // Disabling the module stops its actions without touching them: the
    // run is refused before it starts.
    set_system(&agent, false, true);
    client.request(Request::ReloadConfiguration).await.unwrap();
    let refused = client.request(Request::StartWorkflow { workflow }).await;
    assert!(
        matches!(&refused, Err(ClientError::Agent(e)) if e.message.contains("System module is disabled")),
        "{refused:?}"
    );
    let stored = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    assert!(stored.workflow(workflow).is_some());
    drop(client);
    agent.shutdown().await;
}

/// Runs `git` for test setup in a temporary repository only.
fn test_git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
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
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()
        .unwrap();
    assert!(status.status.success(), "{args:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_reads_the_repository_of_each_runs_own_project() {
    let agent = setup().await;
    // Temporary repositories for CMR and CMS, on separate branches.
    let root = agent.paths.data_dir.join("repositories");
    let mut machine = agent
        .store()
        .load_machine_config()
        .unwrap()
        .unwrap_or_else(|| deckpilot_core::MachineConfig::new(Os::current()));
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    for key in ["CMR", "CMS"] {
        let dir = root.join(key.to_lowercase());
        std::fs::create_dir_all(&dir).unwrap();
        test_git(
            &dir,
            &["init", "-q", "-b", &format!("work-{}", key.to_lowercase())],
        );
        let resource =
            deckpilot_core::ResourceKey::new(format!("{}.repo", key.to_lowercase())).unwrap();
        machine.set_binding(deckpilot_core::MachineResourceBinding {
            key: resource.clone(),
            kind: deckpilot_core::ResourceKind::Directory,
            value: dir.display().to_string(),
        });
        let project = profile
            .projects
            .iter_mut()
            .find(|p| p.key.as_str() == key)
            .unwrap();
        project.resources.insert("repository".into(), resource);
    }
    agent.store().save_machine_config(&machine).unwrap();
    // Each workflow selects its project, waits, then reads "the current
    // project's" repository.
    let status = registry()
        .create_instance(
            "Status",
            deckpilot_modules::git::actions::id(deckpilot_modules::git::actions::STATUS),
            &ActionParameters::new(),
        )
        .unwrap();
    let status_id = status.id;
    profile.actions.push(status);
    let mut workflows = Vec::new();
    for (key, ms) in [("CMR", 300), ("CMS", 50)] {
        let mut workflow = select_wait_announce(
            &mut profile,
            key,
            ms,
            deckpilot_core::ProjectScope::Execution,
        );
        let graph = workflow.graph.as_mut().unwrap();
        for node in &mut graph.nodes {
            if let deckpilot_core::graph::NodeKind::Action { action, .. } = &mut node.kind
                && node.id.as_str() == "announce"
            {
                *action = status_id;
            }
        }
        workflows.push(workflow.id);
        profile.workflows.push(workflow);
    }
    agent.store().save_profile(&profile).unwrap();
    let mut events = agent.client().await;
    events.subscribe().await.unwrap();
    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();

    let first = start_workflow(&mut client, workflows[0]).await;
    let second = start_workflow(&mut client, workflows[1]).await;
    let mut finished = 0;
    while finished < 2 {
        if let AgentEvent::ExecutionFinished { .. } =
            tokio::time::timeout(TIMEOUT, events.next_event())
                .await
                .unwrap()
                .unwrap()
        {
            finished += 1;
        }
    }
    for (id, key, branch) in [
        (first.id, "CMR", "work-cmr"),
        (second.id, "CMS", "work-cms"),
    ] {
        let record = execution(&mut client, id).await;
        let status = node(&record, "announce");
        assert_eq!(
            status.status,
            deckpilot_core::graph::NodeStatus::Succeeded,
            "{:?}",
            status.error
        );
        assert_eq!(status.output["projectKey"], key);
        assert_eq!(status.output["branch"], branch);
        assert_eq!(status.output["unborn"], "true");
    }
    assert_eq!(active_key(&agent.status().await), Some("SHOP"));
    drop((events, client));
    agent.shutdown().await;
}

/// A server on 127.0.0.1 that answers every request with a JSON body and
/// records the requests it received.
fn json_server() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let shared = seen.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            shared.lock().unwrap().push(head);
            let body = r#"{"build":"green","secretInBody":"b0dy-secret"}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (port, seen)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_requests_use_stored_credentials_and_keep_bodies_out_of_history() {
    let agent = setup().await;
    let (port, seen) = json_server();
    agent
        .store()
        .set_credential(
            &deckpilot_core::CredentialId::new("ci.token").unwrap(),
            "CI",
            Some(&deckpilot_core::Secret::new("t0ken-secret")),
        )
        .unwrap();
    agent
        .store()
        .set_module_state(
            &deckpilot_modules::http::module_id(),
            &ModuleSettings {
                enabled: true,
                config: ActionParameters::new().with(
                    deckpilot_modules::http::PRIVATE_HOSTS,
                    deckpilot_core::ParameterValue::Text("127.0.0.1".into()),
                ),
            },
        )
        .unwrap();
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let request = registry()
        .create_instance(
            "Build status",
            deckpilot_modules::http::actions::id(deckpilot_modules::http::actions::REQUEST),
            &ActionParameters::new()
                .with(
                    "url",
                    deckpilot_core::ParameterValue::Text(format!("http://127.0.0.1:{port}/build")),
                )
                .with(
                    "credential",
                    deckpilot_core::ParameterValue::Text("ci.token".into()),
                )
                .with(
                    "extract",
                    deckpilot_core::ParameterValue::Text("/build".into()),
                ),
        )
        .unwrap();
    let mut workflow = deckpilot_core::Workflow::new("Check the build");
    workflow.graph = Some(
        serde_json::from_value(json!({
            "version": 1,
            "nodes": [
                {"id": "start", "type": "trigger"},
                {"id": "request", "type": "action", "action": request.id},
                {"id": "green", "type": "condition", "predicate": {
                    "type": "equals",
                    "reference": {"node": "request", "output": "value"},
                    "value": "green",
                }},
            ],
            "edges": [
                {"from": "start", "to": "request"},
                {"from": "request", "to": "green"},
            ],
        }))
        .unwrap(),
    );
    let workflow_id = workflow.id;
    profile.actions.push(request);
    profile.workflows.push(workflow);
    agent.store().save_profile(&profile).unwrap();
    let mut client = agent.client().await;
    client.subscribe().await.unwrap();
    client.request(Request::ReloadConfiguration).await.unwrap();

    let started = start_workflow(&mut client, workflow_id).await;
    let (events, finished) = execution_events(&mut client, started.id).await;
    assert_eq!(
        finished.state,
        deckpilot_core::ExecutionState::Succeeded,
        "{}",
        finished.message
    );
    assert!(seen.lock().unwrap()[0].contains("authorization: Bearer t0ken-secret"));
    let record = agent.store().execution(started.id).unwrap().unwrap();
    let request = node(&record, "request");
    assert_eq!(request.output["status"], "200");
    // The condition saw the value; history and events have neither the
    // body, the extracted value nor the credential.
    let green = node(&record, "green");
    assert_eq!(green.status, deckpilot_core::graph::NodeStatus::Succeeded);
    let stored = serde_json::to_string(&record).unwrap();
    let live = serde_json::to_string(&events).unwrap();
    for secret in ["t0ken-secret", "b0dy-secret", "\"value\":\"green\""] {
        assert!(!stored.contains(secret), "{secret} in history");
        assert!(!live.contains(secret), "{secret} in events");
    }
    drop(client);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notifications_reach_the_status_bar_and_the_desktop() {
    let paths = temp_paths();
    let (work, key) = seed(&paths);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "TEST-0001");
    let native = std::sync::Arc::new(deckpilot_core::notify::RecordingNotifier::new());
    let (task, stop) = start_agent_with(
        paths.clone(),
        mock.clone(),
        Extras {
            notifier: Some(native.clone()),
            ..Extras::default()
        },
    )
    .unwrap();
    unlock_agent(&paths, &key).await;
    let agent = TestAgent {
        paths,
        mock,
        device,
        work,
        key,
        task: Some(task),
        stop: Some(stop),
    };
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let summary = registry()
        .create_instance(
            "Summary",
            deckpilot_modules::notifications::id(
                deckpilot_modules::notifications::WORKFLOW_SUMMARY,
            ),
            &ActionParameters::new(),
        )
        .unwrap();
    let mut workflow = deckpilot_core::Workflow::new("Start work");
    workflow.graph = Some(
        serde_json::from_value(json!({
            "version": 1,
            "nodes": [
                {"id": "start", "type": "trigger"},
                {"id": "announce", "type": "action", "action": profile.actions[1].id},
                {"id": "summary", "type": "action", "action": summary.id},
            ],
            "edges": [
                {"from": "start", "to": "announce"},
                {"from": "announce", "to": "summary"},
            ],
        }))
        .unwrap(),
    );
    let workflow_id = workflow.id;
    profile.actions.push(summary);
    profile.workflows.push(workflow);
    agent.store().save_profile(&profile).unwrap();

    // The configuration app is open and listening.
    let mut app = AgentClient::connect(&agent.paths, ClientKind::Desktop)
        .await
        .unwrap();
    app.subscribe().await.unwrap();
    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();
    let started = start_workflow(&mut client, workflow_id).await;
    let shown = loop {
        if let AgentEvent::Notification { notification } =
            tokio::time::timeout(TIMEOUT, app.next_event())
                .await
                .unwrap()
                .unwrap()
        {
            break notification;
        }
    };
    assert_eq!(shown.title, "Start work");
    assert!(shown.body.starts_with("1 succeeded"), "{}", shown.body);
    assert_eq!(shown.severity, deckpilot_core::notify::Severity::Success);
    let record = loop {
        let record = execution(&mut client, started.id).await;
        if record.summary.state != deckpilot_core::ExecutionState::Running {
            break record;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let node = node(&record, "summary");
    assert_eq!(node.output["desktop"], "true");
    assert_eq!(node.output["statusBar"], "true");
    assert_eq!(native.shown().len(), 1);
    drop((app, client));
    agent.shutdown().await;
}

/// The image a key of an MK.2 shows for `face`.
fn key_image(face: &deckpilot_core::KeyFace) -> deckpilot_device::Frame {
    let spec = models::stream_deck_mk2()
        .capabilities
        .keys
        .and_then(|keys| keys.image)
        .unwrap();
    deckpilot_agent::render::render(face, spec.width, spec.height)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_workflows_mark_and_redraw_only_their_keys() {
    use deckpilot_core::{FaceStyle, KeyFace};

    let agent = setup().await;
    assert!(agent.frames_shown());
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let workflow = waiting_workflow(&profile, 400);
    let workflow_id = workflow.id;
    profile.workflows.push(workflow);
    let key = ControlId::Key { index: 5 };
    profile.layouts[0].pages[0]
        .bindings
        .push(deckpilot_core::ControlBinding {
            control: key,
            trigger: deckpilot_core::Trigger::Press,
            target: deckpilot_core::BindingTarget::Workflow {
                workflow: workflow_id,
            },
            label: Some("Wait".into()),
            appearance: None,
        });
    agent.store().save_profile(&profile).unwrap();
    let mut events = agent.client().await;
    events.subscribe().await.unwrap();
    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();

    let idle = key_image(&KeyFace::new(FaceStyle::Workflow, "Wait"));
    assert!(agent.mock.wait_for(&agent.device, TIMEOUT, |frames, _| {
        frames.get(&key) == Some(&idle)
    }));
    let before = agent.mock.frames(&agent.device);
    let writes = agent.mock.image_writes(&agent.device);

    let started = start_workflow(&mut client, workflow_id).await;
    // The key shows the run (a running mark and its progress)...
    assert!(agent.mock.wait_for(&agent.device, TIMEOUT, |frames, _| {
        frames.get(&key).is_some_and(|frame| frame != &idle)
    }));
    let (_, finished) = execution_events(&mut events, started.id).await;
    assert_eq!(finished.state, deckpilot_core::ExecutionState::Succeeded);
    // ...and is back to normal once it ended.
    assert!(agent.mock.wait_for(&agent.device, TIMEOUT, |frames, _| {
        frames.get(&key) == Some(&idle)
    }));
    assert_eq!(agent.mock.frames(&agent.device), before);
    // Only that key was redrawn: when the run started, as its nodes
    // finished and when it ended; never the other 14 keys.
    let redrawn = agent.mock.image_writes(&agent.device) - writes;
    assert!((2..=5).contains(&redrawn), "{redrawn} images written");
    agent.shutdown().await;
}

/// The git module with a fake `git` program that logs its calls and
/// reports a clean `main` branch: no repository is read. Returns the log.
#[cfg(unix)]
fn fake_git(agent: &TestAgent) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = agent.paths.data_dir.join("fake-git");
    std::fs::create_dir_all(dir.join("shop")).unwrap();
    let program = dir.join("git");
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\nprintf '# branch.oid \
             1234567890abcdef1234567890abcdef12345678\\0# branch.head main\\0'\n",
            dir.join("calls.log").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    agent
        .store()
        .set_module_state(
            &deckpilot_modules::git::module_id(),
            &ModuleSettings {
                enabled: true,
                config: ActionParameters::new().with(
                    deckpilot_modules::git::EXECUTABLE,
                    deckpilot_core::ParameterValue::Text(program.display().to_string()),
                ),
            },
        )
        .unwrap();
    let mut machine = agent
        .store()
        .load_machine_config()
        .unwrap()
        .unwrap_or_else(|| deckpilot_core::MachineConfig::new(Os::current()));
    machine.set_binding(deckpilot_core::MachineResourceBinding {
        key: deckpilot_core::ResourceKey::new("shop.repository").unwrap(),
        kind: deckpilot_core::ResourceKind::Directory,
        value: dir.join("shop").display().to_string(),
    });
    agent.store().save_machine_config(&machine).unwrap();
    dir.join("calls.log")
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn module_states_are_shown_and_not_refreshed_more_often_than_allowed() {
    use deckpilot_core::{ButtonState, FaceStyle, Indicator, KeyFace};
    use deckpilot_modules::git::actions;

    let agent = setup().await;
    assert!(agent.frames_shown());
    let calls = fake_git(&agent);
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    profile.projects[0].resources.insert(
        "repository".into(),
        deckpilot_core::ResourceKey::new("shop.repository").unwrap(),
    );
    let status = registry()
        .create_instance(
            "Repo",
            actions::id(actions::STATUS),
            &ActionParameters::new(),
        )
        .unwrap();
    let key = ControlId::Key { index: 6 };
    profile.layouts[0].pages[0]
        .bindings
        .push(deckpilot_core::ControlBinding {
            control: key,
            trigger: deckpilot_core::Trigger::Press,
            target: deckpilot_core::BindingTarget::Action { action: status.id },
            label: None,
            appearance: None,
        });
    profile.actions.push(status);
    agent.store().save_profile(&profile).unwrap();
    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();

    // The key shows the branch and a clean work tree once Git answered.
    let ready = key_image(
        &KeyFace::new(FaceStyle::Action, "Repo").with_state(
            &ButtonState::new()
                .with_detail("main")
                .with_indicator(Indicator::Ok),
        ),
    );
    assert!(agent.mock.wait_for(&agent.device, TIMEOUT, |frames, _| {
        frames.get(&key) == Some(&ready)
    }));
    let count = || {
        std::fs::read_to_string(&calls)
            .unwrap_or_default()
            .lines()
            .count()
    };
    assert_eq!(count(), 1);
    let writes = agent.mock.image_writes(&agent.device);

    // Events (key presses that run an action) do not make the provider run
    // again within its interval, and nothing is redrawn.
    for _ in 0..5 {
        agent.mock.tap(&agent.device, ControlId::Key { index: 1 });
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(count(), 1);
    assert_eq!(agent.mock.image_writes(&agent.device), writes);
    assert_eq!(agent.mock.frames(&agent.device)[&key], ready);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_show_their_appearance_and_icons() {
    use deckpilot_core::{Color, FaceStyle, IconFit, IconRef, KeyAppearance, KeyFace, TitleAlign};

    let agent = setup().await;
    assert!(agent.frames_shown());
    let before = agent.mock.frames(&agent.device);
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let png = deckpilot_render::frame_png(&deckpilot_device::Frame::filled(48, 24, [250, 200, 0]))
        .unwrap();
    let icon = profile
        .add_icon(deckpilot_render::icons::import("Sun", &png).unwrap())
        .unwrap();
    let look = KeyAppearance {
        background: Some(Color([20, 40, 60])),
        text_color: Some(Color([255, 255, 0])),
        title_align: Some(TitleAlign::Bottom),
        icon: Some(IconRef {
            id: icon,
            fit: IconFit::Fit,
        }),
        ..KeyAppearance::default()
    };
    // Key 1 runs the log action ("Announce").
    profile.layouts[0].pages[0].bindings[1].appearance = Some(look.clone());
    agent.store().save_profile(&profile).unwrap();
    agent
        .client()
        .await
        .request(Request::ReloadConfiguration)
        .await
        .unwrap();

    let key = ControlId::Key { index: 1 };
    let face = KeyFace::new(FaceStyle::Action, "Announce").with_appearance(Some(&look));
    let spec = models::stream_deck_mk2()
        .capabilities
        .keys
        .and_then(|keys| keys.image)
        .unwrap();
    let expected = deckpilot_agent::render::render_with(
        &face,
        spec.width,
        spec.height,
        &deckpilot_agent::render::Icons::from_profile(&profile),
    );
    assert!(agent.mock.wait_for(&agent.device, TIMEOUT, |frames, _| {
        frames.get(&key) == Some(&expected)
    }));
    // The icon is on the key: its yellow in the middle, the background at
    // the corner.
    assert_eq!(
        expected.pixel(spec.width / 2, spec.height / 2 - 4),
        Some([250, 200, 0])
    );
    assert_eq!(expected.pixel(0, 0), Some([20, 40, 60]));
    // Other keys are unchanged.
    let after = agent.mock.frames(&agent.device);
    assert_eq!(
        after[&ControlId::Key { index: 0 }],
        before[&ControlId::Key { index: 0 }]
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn folders_go_back_to_their_parent_with_an_automatic_back_key() {
    use deckpilot_core::{FaceStyle, KeyFace, LayoutPage};

    let agent = setup().await;
    assert!(agent.frames_shown());
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    // Home key 5 opens "Folder"; "Folder" key 5 opens "Inner"; neither has
    // a Back key (an older layout).
    let folder = LayoutPage::new("Folder");
    let inner = LayoutPage::new("Inner");
    let (folder_id, inner_id) = (folder.id, inner.id);
    let open = |page| {
        deckpilot_core::ControlBinding::new(
            ControlId::Key { index: 5 },
            deckpilot_core::Trigger::Press,
            deckpilot_core::BindingTarget::OpenPage { page },
        )
    };
    let mut folder = folder;
    folder.bindings.push(open(inner_id));
    profile.layouts[0].pages[0].bindings.push(open(folder_id));
    profile.layouts[0].pages.extend([folder, inner]);
    agent.store().save_profile(&profile).unwrap();
    let mut client = agent.client().await;
    client.request(Request::ReloadConfiguration).await.unwrap();

    let key = |index| ControlId::Key { index };
    let folder_face = key_image(&KeyFace::new(FaceStyle::Folder, "Folder"));
    let back = key_image(&KeyFace::new(FaceStyle::Back, "Back"));
    assert!(agent.mock.wait_for(&agent.device, TIMEOUT, |frames, _| {
        frames.get(&key(5)) == Some(&folder_face)
    }));
    let home_frames = agent.mock.frames(&agent.device);
    let page = |status: &AgentStatus| status.devices[0].page.clone();

    agent.mock.tap(&agent.device, key(5));
    agent
        .wait_for_status(|s| page(s).as_deref() == Some("Folder"))
        .await;
    agent.mock.tap(&agent.device, key(5));
    agent
        .wait_for_status(|s| page(s).as_deref() == Some("Inner"))
        .await;
    // The first free key is an automatic Back.
    assert!(agent.mock.wait_for(&agent.device, TIMEOUT, |frames, _| {
        frames.get(&key(0)) == Some(&back)
    }));
    // Back goes to the parent folder, then home.
    agent.mock.tap(&agent.device, key(0));
    agent
        .wait_for_status(|s| page(s).as_deref() == Some("Folder"))
        .await;
    agent.mock.tap(&agent.device, key(0));
    agent
        .wait_for_status(|s| page(s).as_deref() == Some("Home"))
        .await;
    assert!(agent.mock.wait_for(&agent.device, TIMEOUT, |frames, _| {
        *frames == home_frames
    }));
    // The stored layout was not rewritten by any of this.
    let stored = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    assert_eq!(stored, profile);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connections_offer_only_what_their_protocol_has() {
    let agent = setup().await;
    // The current client gets everything.
    let current = agent.client().await;
    assert_eq!(current.protocol(), PROTOCOL_VERSION);
    assert_eq!(
        current.capabilities(),
        deckpilot_ipc::Capability::of_protocol(PROTOCOL_VERSION)
    );
    // A 0.5.0 client (protocol 4) is welcomed with protocol 4's capabilities
    // and refused what it could not know, even if it sent it anyway.
    let paths = agent.paths.clone();
    let stream = paths.endpoint().connect().await.unwrap();
    let stream = SharedStream::new(stream);
    let mut reader = BufReader::new(stream.clone());
    let mut writer = stream;
    let token = std::fs::read_to_string(&paths.token_file).unwrap();
    write_message(
        &mut writer,
        &json!({"type": "hello", "protocol": 4, "token": token.trim(), "client": "desktop"}),
    )
    .await
    .unwrap();
    let welcome: Value = read_message(&mut reader).await.unwrap().unwrap();
    assert_eq!(welcome["protocol"], PROTOCOL_VERSION);
    assert_eq!(
        welcome["capabilities"],
        json!([
            "workflows",
            "prepareForUpdate",
            "moduleNamespaces",
            "encryptedDatabase"
        ])
    );
    write_message(
        &mut writer,
        &json!({"type": "request", "id": 1, "request": {"type": "listExecutions"}}),
    )
    .await
    .unwrap();
    let refused: Value = read_message(&mut reader).await.unwrap().unwrap();
    assert_eq!(refused["error"]["code"], "unsupported", "{refused}");
    // The status is still served, with the running count newer clients use.
    write_message(
        &mut writer,
        &json!({"type": "request", "id": 2, "request": {"type": "getStatus"}}),
    )
    .await
    .unwrap();
    let status: Value = read_message(&mut reader).await.unwrap().unwrap();
    assert_eq!(status["response"]["status"]["runningExecutions"], 0);
    drop((reader, writer, current));
    agent.shutdown().await;
}

/// A cancelled run whose action does not stop within the cancel grace is
/// recorded as cancelled at the end of the grace. Subscribers then learn
/// at once that nothing runs any more (the tray, the app, keys showing
/// progress), and the result the action delivers later changes nothing.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_that_outlives_its_cancel_grace_ends_for_everyone() {
    let agent = setup_with(Extras {
        cancel_grace: Some(Duration::from_millis(300)),
        ..Extras::default()
    })
    .await;
    set_system(&agent, true, true);
    // Ignores the polite stop; it is killed only after the process grace
    // (2 s), well after the agent's cancel grace.
    let workflow = command_workflow(&agent, "trap '' TERM; sleep 4");
    let mut client = agent.client().await;
    client.subscribe().await.unwrap();
    client.request(Request::ReloadConfiguration).await.unwrap();

    let run = start_workflow(&mut client, workflow).await;
    agent
        .wait_for_status(|s| s.running_executions == Some(1))
        .await;
    // Let the shell install its trap before it is asked to stop.
    tokio::time::sleep(Duration::from_millis(300)).await;
    client
        .request(Request::CancelExecution { execution: run.id })
        .await
        .unwrap();
    let (_, finished) = execution_events(&mut client, run.id).await;
    assert_eq!(finished.state, deckpilot_core::ExecutionState::Cancelled);
    assert!(
        finished.message.contains("cannot be interrupted"),
        "{}",
        finished.message
    );
    // A state update follows right away, not only with the next change.
    let status = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let AgentEvent::State { status } = client.next_event().await.unwrap() {
                break status;
            }
        }
    })
    .await
    .expect("a state update after the run ended");
    assert_eq!(status.running_executions, Some(0));

    // The command is killed after its grace; its late result is discarded.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let record = agent.store().execution(run.id).unwrap().unwrap();
    assert_eq!(
        record.summary.state,
        deckpilot_core::ExecutionState::Cancelled
    );
    assert_eq!(agent.status().await.running_executions, Some(0));
    // The workflow can run again.
    let again = start_workflow(&mut client, workflow).await;
    client
        .request(Request::CancelExecution {
            execution: again.id,
        })
        .await
        .unwrap();
    agent.shutdown().await;
}

/// At most four workflows run together; a fifth is refused with a reason
/// and recorded nowhere, and starts once one of the four has ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn at_most_four_workflows_run_together() {
    let agent = setup().await;
    let mut profile = agent.store().load_profile(agent.work.id).unwrap().unwrap();
    let wait = registry()
        .create_instance(
            "Wait a while",
            general::wait_id(),
            &ActionParameters::new().with("seconds", deckpilot_core::ParameterValue::Integer(300)),
        )
        .unwrap();
    let wait_id = wait.id;
    profile.actions.push(wait);
    let workflows: Vec<deckpilot_core::WorkflowId> = (0..5)
        .map(|n| {
            let mut workflow = deckpilot_core::Workflow::new(format!("Wait {n}"));
            workflow.graph = Some(
                serde_json::from_value(json!({
                    "version": 1,
                    "nodes": [
                        {"id": "start", "type": "trigger"},
                        {"id": "wait", "type": "action", "action": wait_id},
                    ],
                    "edges": [{"from": "start", "to": "wait"}],
                }))
                .unwrap(),
            );
            let id = workflow.id;
            profile.workflows.push(workflow);
            id
        })
        .collect();
    agent.store().save_profile(&profile).unwrap();
    let mut client = agent.client().await;
    client.subscribe().await.unwrap();
    client.request(Request::ReloadConfiguration).await.unwrap();

    let mut runs = Vec::new();
    for workflow in &workflows[..4] {
        runs.push(start_workflow(&mut client, *workflow).await);
    }
    let refused = client
        .request(Request::StartWorkflow {
            workflow: workflows[4],
        })
        .await;
    assert!(
        matches!(&refused, Err(ClientError::Agent(e)) if e.message.contains("4 workflows are already running")),
        "{refused:?}"
    );
    assert_eq!(agent.status().await.running_executions, Some(4));
    let listed = |executions: &[deckpilot_core::ExecutionSummary]| {
        executions
            .iter()
            .filter(|e| e.workflow == workflows[4])
            .count()
    };
    let Response::Executions { executions } = client
        .request(Request::ListExecutions {
            workflow: None,
            limit: Some(50),
        })
        .await
        .unwrap()
    else {
        panic!("executions expected");
    };
    assert_eq!(executions.len(), 4);
    assert_eq!(listed(&executions), 0, "a refused start leaves no record");

    // One ends: the fifth starts.
    client
        .request(Request::CancelExecution {
            execution: runs[0].id,
        })
        .await
        .unwrap();
    let (_, finished) = execution_events(&mut client, runs[0].id).await;
    assert_eq!(finished.state, deckpilot_core::ExecutionState::Cancelled);
    let fifth = start_workflow(&mut client, workflows[4]).await;
    assert_eq!(agent.status().await.running_executions, Some(4));

    // Each run has its own record; stopping the agent ends the others as
    // interrupted.
    let store = agent.store();
    agent.shutdown().await;
    let state = |id| store.execution(id).unwrap().unwrap().summary.state;
    assert_eq!(state(runs[0].id), deckpilot_core::ExecutionState::Cancelled);
    for run in runs[1..].iter().chain([&fifth]) {
        assert_eq!(state(run.id), deckpilot_core::ExecutionState::Interrupted);
    }
}
