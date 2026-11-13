//! The agent executable with `--mock-devices mk2`, driven only over IPC as
//! the configuration app and `deckpilot-dev hardware-test --mock` drive it:
//! folder navigation with the automatic Back key, a workflow started by a
//! key press, and profile switching. Plus, in process, key icons following
//! a profile switch. Simulated devices and temporary data only.

mod support;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use deckpilot_core::{
    ActionParameters, BindingTarget, Color, ControlBinding, DeviceLayout, ExecutionState,
    ExecutionTrigger, FaceStyle, IconFit, IconRef, KeyAppearance, KeyFace, LayoutPage,
    LayoutStatus, ParameterValue, Profile, Project, ProjectKey, Trigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, Frame, models};
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, AgentStatus, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_modules::general;
use deckpilot_modules::projects::{ProjectCatalog, actions};
use deckpilot_storage::ConfigStore;
use support::{TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn press(control: ControlId) -> ControlBinding {
    ControlBinding::new(control, Trigger::Press, BindingTarget::Back)
}

/// A log action named `name`.
fn log_action(profile: &mut Profile, name: &str) -> BindingTarget {
    let action = support::registry()
        .create_instance(
            name,
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text(name.into())),
        )
        .unwrap();
    let target = BindingTarget::action(action.id);
    profile.actions.push(action);
    target
}

/// An icon of one colour, added to `profile`.
fn icon(profile: &mut Profile, name: &str, color: [u8; 3]) -> KeyAppearance {
    let png = deckpilot_render::frame_png(&Frame::filled(40, 40, color)).unwrap();
    let id = profile
        .add_icon(deckpilot_render::icons::import(name, &png).unwrap())
        .unwrap();
    KeyAppearance {
        background: Some(Color([10, 10, 10])),
        icon: Some(IconRef {
            id,
            fit: IconFit::Fit,
        }),
        ..KeyAppearance::default()
    }
}

/// "Work": SHOP and CMR; key 0 next project, key 1 "A key" with a yellow
/// icon, key 3 the "Build" workflow, key 5 opens "Folder" (key 2 there:
/// "In folder"; no Back key, so key 0 becomes an automatic Back).
fn work() -> Profile {
    let mut profile = Profile::new("Work");
    for code in ["SHOP", "CMR"] {
        profile
            .add_project(Project::new(ProjectKey::new(code).unwrap(), code))
            .unwrap();
    }
    profile.default_project = Some(profile.projects[0].id);
    let next = support::registry()
        .create_instance("Next project", actions::next_id(), &ActionParameters::new())
        .unwrap();
    let next_target = BindingTarget::action(next.id);
    profile.actions.push(next);
    let a_key = log_action(&mut profile, "A key");
    let in_folder = log_action(&mut profile, "In folder");
    let look = icon(&mut profile, "Sun", [250, 200, 0]);
    let workflow = support::waiting_workflow("Build", 10);
    let workflow_target = BindingTarget::Workflow {
        workflow: workflow.id,
    };
    profile.workflows.push(workflow);

    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let mut folder = LayoutPage::new("Folder");
    folder.bindings.push(ControlBinding {
        target: in_folder,
        ..press(key(2))
    });
    let home = &mut layout.pages[0];
    home.bindings.extend([
        ControlBinding {
            target: next_target,
            ..press(key(0))
        },
        ControlBinding {
            target: a_key,
            appearance: Some(look),
            ..press(key(1))
        },
        ControlBinding {
            target: workflow_target,
            ..press(key(3))
        },
        ControlBinding {
            target: BindingTarget::OpenPage { page: folder.id },
            ..press(key(5))
        },
    ]);
    layout.pages.push(folder);
    profile.layouts.push(layout);
    profile
}

/// "Home": key 0 "B key", key 1 "B icon" with a blue icon.
fn home() -> Profile {
    let mut profile = Profile::new("Home");
    profile
        .add_project(Project::new(ProjectKey::new("HOME").unwrap(), "Home"))
        .unwrap();
    let b_key = log_action(&mut profile, "B key");
    let b_icon = log_action(&mut profile, "B icon");
    let look = icon(&mut profile, "Sea", [0, 90, 250]);
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout.pages[0].bindings.extend([
        ControlBinding {
            target: b_key,
            ..press(key(0))
        },
        ControlBinding {
            target: b_icon,
            appearance: Some(look),
            ..press(key(1))
        },
    ]);
    profile.layouts.push(layout);
    profile
}

/// A data directory with both profiles, "Work" active.
fn data(dir: &TempDir) -> (AgentPaths, deckpilot_storage::DatabaseKey, Profile, Profile) {
    let (work, home) = (work(), home());
    let (paths, key) = support::data_dir(dir, &work, &[]);
    support::open_store(&paths, &key)
        .save_profile(&home)
        .unwrap();
    (paths, key, work, home)
}

/// How long the executable may take to start and accept connections; a
/// cold start on a CI runner (Windows in particular) can be slow.
const STARTUP: Duration = Duration::from_secs(90);

/// The agent executable, stopped (and killed if needed) when dropped. All
/// requests go over one connection, so a Windows named pipe is not opened
/// and closed for every request.
struct AgentProcess {
    child: Child,
    paths: AgentPaths,
    /// The executable's standard error, shown when something goes wrong.
    stderr: PathBuf,
    client: Option<AgentClient>,
}

impl AgentProcess {
    /// Starts the executable and waits until it accepts a connection.
    async fn start(paths: &AgentPaths, dir: &TempDir) -> Self {
        let stderr = dir.path().join("agent-stderr.log");
        let child = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
            .args(["--headless", "--mock-devices", "mk2"])
            .env(deckpilot_ipc::paths::DATA_DIR_ENV, &paths.data_dir)
            .env("RUST_BACKTRACE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let mut agent = Self {
            child,
            paths: paths.clone(),
            stderr,
            client: None,
        };
        let client = agent.connect(ClientKind::Desktop).await;
        agent.client = Some(client);
        agent
    }

    /// What the executable reported: its exit status, standard error and
    /// log files.
    fn diagnostics(&mut self) -> String {
        let status = match self.child.try_wait() {
            Ok(Some(status)) => format!("exited with {status}"),
            Ok(None) => "still running".to_owned(),
            Err(error) => format!("unknown ({error})"),
        };
        let mut text = format!(
            "agent {status}\n--- stderr ---\n{}",
            std::fs::read_to_string(&self.stderr).unwrap_or_default()
        );
        if let Ok(entries) = std::fs::read_dir(&self.paths.log_dir) {
            for entry in entries.flatten() {
                let contents = std::fs::read_to_string(entry.path()).unwrap_or_default();
                let tail: Vec<&str> = contents.lines().rev().take(60).collect();
                let _ = writeln!(text, "\n--- {} ---", entry.path().display());
                for line in tail.into_iter().rev() {
                    text.push_str(line);
                    text.push('\n');
                }
            }
        }
        text
    }

    /// A new connection, retried until the agent accepts it.
    async fn connect(&mut self, kind: ClientKind) -> AgentClient {
        let deadline = tokio::time::Instant::now() + STARTUP;
        loop {
            let error = match AgentClient::connect(&self.paths, kind).await {
                Ok(client) => return client,
                Err(error) => error,
            };
            if matches!(self.child.try_wait(), Ok(Some(_)))
                || tokio::time::Instant::now() > deadline
            {
                panic!(
                    "cannot connect to the agent (last error: {error})\n{}",
                    self.diagnostics()
                );
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn request(&mut self, request: Request) -> Response {
        let client = self.client.as_mut().expect("connected");
        match client.request(request).await {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}\n{}", self.diagnostics()),
        }
    }

    async fn wait_status(&mut self, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            if let Response::Status { status } = self.request(Request::GetStatus).await
                && predicate(&status)
            {
                return *status;
            }
            assert!(
                tokio::time::Instant::now() <= deadline,
                "condition not reached in time\n{}",
                self.diagnostics()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn tap(&mut self, device: &DeviceId, control: ControlId) {
        for input in [ControlInput::Press, ControlInput::Release] {
            self.request(Request::SimulateInput {
                device: device.clone(),
                control,
                input,
            })
            .await;
        }
    }

    async fn stop(mut self) {
        self.request(Request::Shutdown).await;
        self.client = None;
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        while self.child.try_wait().unwrap().is_none() {
            assert!(
                tokio::time::Instant::now() <= deadline,
                "the agent did not stop\n{}",
                self.diagnostics()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The next event matching `select`, skipping others.
async fn next<T>(events: &mut AgentClient, select: impl Fn(AgentEvent) -> Option<T>) -> T {
    loop {
        let event = tokio::time::timeout(TIMEOUT, events.next_event())
            .await
            .expect("event in time")
            .unwrap();
        if let Some(value) = select(event) {
            return value;
        }
    }
}

fn completed(event: AgentEvent) -> Option<String> {
    match event {
        AgentEvent::ActionCompleted { name, .. } => Some(name),
        _ => None,
    }
}

fn page(status: &AgentStatus) -> Option<&str> {
    status.devices.first().and_then(|d| d.page.as_deref())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_mock_agent_navigates_runs_workflows_and_switches_profiles() {
    let dir = TempDir::new("deckpilot-mock");
    let (paths, key_material, work, home) = data(&dir);
    let mut agent = AgentProcess::start(&paths, &dir).await;
    agent
        .request(Request::Unlock {
            key: KeyMaterial::new(key_material.to_hex()),
        })
        .await;
    let status = agent
        .wait_status(|s| {
            s.devices
                .iter()
                .any(|d| d.state == ConnectionState::Connected && d.page.is_some())
        })
        .await;
    let device = status.devices[0].id.clone();
    assert_eq!(device.serial, "SIM-MK2-01");
    assert_eq!(status.devices[0].source, "mock");
    assert!(matches!(
        status.devices[0].layout,
        LayoutStatus::Compatible { .. }
    ));
    assert_eq!(page(&status), Some("Home"));
    let mut events = agent.connect(ClientKind::Other).await;
    events.subscribe().await.unwrap();

    // Folder, its own key, then the automatic Back on the first free key.
    agent.tap(&device, key(5)).await;
    agent.wait_status(|s| page(s) == Some("Folder")).await;
    agent.tap(&device, key(2)).await;
    assert_eq!(next(&mut events, completed).await, "In folder");
    agent.tap(&device, key(0)).await;
    agent.wait_status(|s| page(s) == Some("Home")).await;

    // A key bound to a workflow starts it in the background.
    let workflow = work.workflows[0].id;
    agent.tap(&device, key(3)).await;
    let started = next(&mut events, |event| match event {
        AgentEvent::ExecutionStarted { execution } => Some(execution),
        _ => None,
    })
    .await;
    assert_eq!(started.workflow, workflow);
    assert_eq!(started.trigger, ExecutionTrigger::Device);
    let finished = next(&mut events, |event| match event {
        AgentEvent::ExecutionFinished { execution } if execution.id == started.id => {
            Some(execution)
        }
        _ => None,
    })
    .await;
    assert_eq!(finished.state, ExecutionState::Succeeded);

    // Another profile: its layout, home page and actions; nothing runs by
    // switching.
    agent.tap(&device, key(5)).await;
    agent.wait_status(|s| page(s) == Some("Folder")).await;
    agent
        .request(Request::SetActiveProfile { profile: home.id })
        .await;
    let status = agent
        .wait_status(|s| s.active_profile.as_ref().is_some_and(|p| p.id == home.id))
        .await;
    assert_eq!(page(&status), Some("Home"));
    agent.tap(&device, key(0)).await;
    assert_eq!(next(&mut events, completed).await, "B key");

    // And back: key 0 is the project switcher again.
    agent
        .request(Request::SetActiveProfile { profile: work.id })
        .await;
    agent
        .wait_status(|s| s.active_profile.as_ref().is_some_and(|p| p.id == work.id))
        .await;
    agent.tap(&device, key(0)).await;
    assert_eq!(next(&mut events, completed).await, "Next project");
    agent
        .wait_status(|s| {
            s.active_project
                .as_ref()
                .is_some_and(|p| p.key.as_str() == "CMR")
        })
        .await;
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_icons_follow_the_active_profile() {
    let dir = TempDir::new("deckpilot-mock");
    let (paths, key_material, work, home) = data(&dir);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;

    let spec = models::stream_deck_mk2()
        .capabilities
        .keys
        .and_then(|keys| keys.image)
        .unwrap();
    let expected = |profile: &Profile, title: &str| {
        let look = profile.layouts[0].pages[0]
            .bindings
            .iter()
            .find(|b| b.control == key(1))
            .and_then(|b| b.appearance.clone());
        let face = KeyFace::new(FaceStyle::Action, title).with_appearance(look.as_ref());
        deckpilot_agent::render::render_with(
            &face,
            spec.width,
            spec.height,
            &deckpilot_agent::render::Icons::from_profile(profile),
        )
    };
    let (sun, sea) = (expected(&work, "A key"), expected(&home, "B icon"));
    let centre = (spec.width / 2, spec.height / 2 - 4);
    assert_eq!(sun.pixel(centre.0, centre.1), Some([250, 200, 0]));
    assert_eq!(sea.pixel(centre.0, centre.1), Some([0, 90, 250]));
    let shows = |frame: &Frame| {
        mock.wait_for(&device, TIMEOUT, |frames, _| {
            frames.get(&key(1)) == Some(frame)
        })
    };

    assert!(shows(&sun), "Work's icon");
    agent
        .request(Request::SetActiveProfile { profile: home.id })
        .await
        .unwrap();
    assert!(shows(&sea), "Home's icon after switching");
    agent
        .request(Request::SetActiveProfile { profile: work.id })
        .await
        .unwrap();
    assert!(shows(&sun), "Work's icon again");
    agent.shutdown().await;
}

/// The status of `agent` once `predicate` holds.
async fn wait_until(agent: &TestAgent, predicate: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        if predicate(&status) {
            return status;
        }
        assert!(
            tokio::time::Instant::now() <= deadline,
            "condition not reached in time: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// First run (defect D1): the devices are connected before any profile or
/// layout exists. A layout added to the active profile afterwards, saved
/// as the configuration app saves it, reaches the connected devices: their
/// home page is shown and their keys run actions, without switching
/// profiles again, refreshing devices or restarting the agent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_layout_added_to_the_active_profile_reaches_connected_devices() {
    let dir = TempDir::new("deckpilot-mock");
    let paths = AgentPaths::in_dir(dir.path().join("data"));
    paths.create_dirs().unwrap();
    let key_material = support::create_database(&paths);
    let mock = MockBackend::new();
    let first = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let second = mock.plug(&models::stream_deck_mk2(), "MK2-B");
    let agent = TestAgent::start_unlocked(&paths, &key_material, &mock, None).await;
    let status = wait_until(&agent, |s| {
        s.devices
            .iter()
            .filter(|d| d.state == ConnectionState::Connected)
            .count()
            == 2
    })
    .await;
    assert!(status.active_profile.is_none());
    assert!(status.devices.iter().all(|d| d.page.is_none()));

    // A new profile, made active as the configuration app creates it.
    let profile = Profile::new("Work");
    let store = agent.store();
    store.save_profile(&profile).unwrap();
    store.set_active_profile(Some(profile.id)).unwrap();
    agent.request(Request::ReloadConfiguration).await.unwrap();
    let status = wait_until(&agent, |s| {
        s.active_profile
            .as_ref()
            .is_some_and(|p| p.id == profile.id)
    })
    .await;
    assert!(status.devices.iter().all(|d| d.page.is_none()));

    // An MK.2 layout with key 0 bound to an action, saved and activated.
    agent
        .edit_profile(profile.id, |profile| {
            let target = log_action(profile, "A key");
            let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
            layout.pages[0].bindings.push(ControlBinding {
                target,
                ..press(key(0))
            });
            let id = layout.id;
            profile.layouts.push(layout);
            profile.activate_layout(id).unwrap();
        })
        .await;
    let status = wait_until(&agent, |s| {
        s.devices.iter().all(|d| d.page.as_deref() == Some("Home"))
    })
    .await;
    assert!(
        status
            .devices
            .iter()
            .all(|d| matches!(d.layout, LayoutStatus::Compatible { .. }))
    );

    // Pressing key 0 on either device runs the action.
    let mut events = agent.events().await;
    for device in [&first, &second] {
        for input in [ControlInput::Press, ControlInput::Release] {
            agent
                .request(Request::SimulateInput {
                    device: device.clone(),
                    control: key(0),
                    input,
                })
                .await
                .unwrap();
        }
        assert_eq!(next(&mut events, completed).await, "A key");
    }
    agent.shutdown().await;
}
