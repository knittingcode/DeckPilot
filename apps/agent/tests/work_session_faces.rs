//! Faces of work session keys (`docs/gestures-and-work-session.md`,
//! section 8.2, W4) on a simulated MK.2: the face of each state, and the
//! elapsed-time redraw, which is scheduled only while a work session key
//! is shown on a connected device and its timer runs.
//!
//! The agent's wall clock is real time moved by an offset, so a test can
//! let minutes of elapsed time pass at once; the session is operated
//! through the `test-hooks` entry point, as a key gesture would.

mod support;

use std::time::{Duration, Instant};

use deckpilot_agent::{WorkSessionOperation as Op, WorkSessionOutcome};
use deckpilot_core::gesture::{Gesture, KeyGestureSettings};
use deckpilot_core::graph::{ValueKind, WorkflowOutput};
use deckpilot_core::stateful::StatefulBinding;
use deckpilot_core::work_session::{WorkSessionSnapshot, WorkSessionState as S};
use deckpilot_core::{
    BindingTarget, Color, ControlBinding, DeviceLayout, FaceStyle, Indicator, KeyAppearance,
    KeyFace, LayoutPage, Profile, Trigger, Workflow,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, DeviceId, models};
use deckpilot_ipc::{AgentPaths, Request};
use deckpilot_storage::DatabaseKey;
use deckpilot_storage::work_sessions::{ElapsedFormat, WorkSessionSettings};
use serde_json::json;
use support::{
    Jump, TIMEOUT, TempDir, TestAgent, data_dir, graph, graph_workflow, waiting_workflow,
};

/// The work session key.
const KEY: ControlId = ControlId::Key { index: 0 };
/// Opens the folder page, where the work session key is not shown.
const FOLDER: ControlId = ControlId::Key { index: 1 };
const MINUTE: i64 = 60_000;
/// The key's own colour, kept on every face.
const BACKGROUND: Color = Color([10, 60, 30]);

fn appearance() -> KeyAppearance {
    KeyAppearance {
        background: Some(BACKGROUND),
        ..KeyAppearance::default()
    }
}

/// Key 0 follows the work session (with its own background); key 1 opens
/// a folder, whose automatic Back key is key 0.
fn work_profile(workflows: Vec<Workflow>) -> Profile {
    let mut profile = Profile::new("Work");
    profile.workflows = workflows;
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let folder = LayoutPage::new("Folder");
    let folder_id = folder.id;
    let page = &mut layout.pages[0];
    let session = || BindingTarget::Stateful(StatefulBinding::work_session());
    let mut click = ControlBinding::new(KEY, Trigger::Click, session());
    click.appearance = Some(appearance());
    page.bindings = vec![
        click,
        ControlBinding::new(KEY, Trigger::LongPress, session()),
        ControlBinding::new(
            FOLDER,
            Trigger::Press,
            BindingTarget::OpenPage { page: folder_id },
        ),
    ];
    page.gestures = vec![KeyGestureSettings {
        control: KEY,
        double_click_ms: None,
        long_press_ms: Some(300),
    }];
    layout.pages.push(folder);
    profile.layouts.push(layout);
    profile
}

/// `start → wait → out`: answers `canFinish = true` after `wait_ms`.
fn verification(wait_ms: u32) -> Workflow {
    let mut workflow = graph_workflow(
        "Check",
        graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "wait", "type": "delay", "ms": wait_ms},
                {"id": "out", "type": "output", "values": {
                    "canFinish": {"type": "template", "template": "true"}
                }},
            ]),
            &json!([{"from": "start", "to": "wait"}, {"from": "wait", "to": "out"}]),
        ),
    );
    workflow.outputs = vec![WorkflowOutput::new("canFinish", ValueKind::Boolean)];
    workflow
}

struct Fixture {
    _dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
    mock: MockBackend,
    device: DeviceId,
    profile: Profile,
    jump: Jump,
}

impl Fixture {
    fn new(workflows: Vec<Workflow>) -> Self {
        let dir = TempDir::new("deckpilot-work-session-faces");
        let profile = work_profile(workflows);
        let (paths, key) = data_dir(&dir, &profile, &[]);
        let mock = MockBackend::new();
        let device = mock.plug(&models::stream_deck_mk2(), "FACES-1");
        Self {
            _dir: dir,
            paths,
            key,
            mock,
            device,
            profile,
            jump: Jump::default(),
        }
    }

    async fn agent(&self) -> TestAgent {
        let agent =
            TestAgent::start_unlocked(&self.paths, &self.key, &self.mock, Some(self.jump.clock()))
                .await;
        self.wait_connected(&agent, true).await;
        agent
    }

    async fn wait_connected(&self, agent: &TestAgent, wanted: bool) {
        let deadline = Instant::now() + TIMEOUT;
        while connected(agent, &self.device).await != wanted {
            assert!(
                Instant::now() < deadline,
                "the MK.2 connection did not change"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn configure(&self, agent: &TestAgent, change: impl FnOnce(&mut WorkSessionSettings)) {
        let store = agent.store();
        let mut settings = store.work_session_settings(self.profile.id).unwrap();
        change(&mut settings);
        store
            .save_work_session_settings(&settings, self.jump.now_ms())
            .unwrap();
        agent.request(Request::ReloadConfiguration).await.unwrap();
    }

    fn image(face: &KeyFace) -> deckpilot_device::Frame {
        let spec = models::stream_deck_mk2()
            .capabilities
            .keys
            .and_then(|keys| keys.image)
            .unwrap();
        deckpilot_agent::render::render(face, spec.width, spec.height)
    }

    /// Waits until key 0 shows `face` and `writes` images were written in
    /// all (if given); returns the number of writes.
    fn wait_face(&self, face: &KeyFace, writes: Option<usize>) -> usize {
        let image = Self::image(face);
        assert!(
            self.mock
                .wait_for(&self.device, TIMEOUT, |frames, written| {
                    frames.get(&KEY) == Some(&image) && writes.is_none_or(|w| w == written)
                }),
            "key 0 does not show {face:?} after {writes:?} writes (now {})",
            self.writes()
        );
        self.writes()
    }

    fn writes(&self) -> usize {
        self.mock.image_writes(&self.device)
    }
}

async fn connected(agent: &TestAgent, device: &DeviceId) -> bool {
    agent
        .status()
        .await
        .devices
        .iter()
        .any(|d| &d.id == device && d.state == ConnectionState::Connected)
}

async fn op(agent: &TestAgent, operation: Op, gesture: Gesture) {
    let outcome = agent
        .handle
        .work_session_operation(operation, gesture, None)
        .await
        .expect("the agent answers");
    assert_eq!(outcome, WorkSessionOutcome::Applied, "{operation:?}");
}

async fn wait_state(agent: &TestAgent, state: S) -> WorkSessionSnapshot {
    let mut updates = agent.handle.work_session();
    let reached = tokio::time::timeout(TIMEOUT, async {
        loop {
            {
                let current = updates.borrow_and_update();
                if current.state == state {
                    return current.clone();
                }
            }
            updates.changed().await.expect("the agent runs");
        }
    })
    .await;
    reached.unwrap_or_else(|_| panic!("{} not reached", state.name()))
}

/// The work session key's face.
fn face(title: &str, detail: Option<&str>, indicator: Option<Indicator>) -> KeyFace {
    KeyFace {
        detail: detail.map(str::to_owned),
        indicator,
        ..KeyFace::new(FaceStyle::Action, title).with_appearance(Some(&appearance()))
    }
}

fn working(elapsed: &str) -> KeyFace {
    face(elapsed, Some("Working"), Some(Indicator::Ok))
}

/// Lets the agent run a while; nothing is due in that time.
async fn idle_for(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_state_shows_its_face() {
    let start = waiting_workflow("Start", 1500);
    let check = verification(1500);
    let (start_id, check_id) = (start.id, check.id);
    let f = Fixture::new(vec![start, check]);
    let agent = f.agent().await;
    f.configure(&agent, |s| {
        s.start_workflow = Some(start_id);
        s.verification_workflow = Some(check_id);
    })
    .await;

    f.wait_face(&face("Start Work", None, None), None);
    // Nothing ticks while idle.
    assert_eq!(agent.handle.work_session_face_due().await, None);

    op(&agent, Op::Start, Gesture::Click).await;
    f.wait_face(&face("Starting…", None, Some(Indicator::Running)), None);
    assert_eq!(agent.handle.work_session_face_due().await, None);

    wait_state(&agent, S::Working).await;
    f.wait_face(&working("0:00"), None);
    assert!(agent.handle.work_session_face_due().await.is_some());

    op(&agent, Op::Verify, Gesture::Click).await;
    f.wait_face(
        &face("Checking…", Some("0:00"), Some(Indicator::Running)),
        None,
    );

    wait_state(&agent, S::ReadyToFinish).await;
    f.wait_face(
        &face("Finish Work", Some("0:00"), Some(Indicator::Ok)),
        None,
    );

    // Past the arming delay, a click finishes; stopping completes at once.
    idle_for(1200).await;
    op(&agent, Op::Finish, Gesture::Click).await;
    wait_state(&agent, S::Idle).await;
    f.wait_face(&face("Start Work", None, None), None);
    assert_eq!(agent.handle.work_session_face_due().await, None);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn elapsed_time_redraws_only_that_key_once_per_minute() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    op(&agent, Op::Start, Gesture::Click).await;
    wait_state(&agent, S::Working).await;
    let before = f.wait_face(&working("0:00"), None);
    // The next redraw is at the next minute of elapsed time.
    let due = agent.handle.work_session_face_due().await.unwrap();
    let left = due - f.jump.now_ms();
    assert!((0..=MINUTE).contains(&left), "{left} ms");

    // Three minutes pass: three images, all of key 0.
    for minute in 1..=3 {
        f.jump.by(MINUTE);
        f.wait_face(&working(&format!("0:0{minute}")), Some(before + minute));
    }
    let frames = f.mock.frames(&f.device);
    idle_for(1000).await;
    assert_eq!(f.writes(), before + 3, "only the minute boundaries redraw");
    assert_eq!(f.mock.frames(&f.device), frames);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_page_switch_or_a_disconnect_stops_the_redraws() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    op(&agent, Op::Start, Gesture::Click).await;
    wait_state(&agent, S::Working).await;
    f.wait_face(&working("0:00"), None);
    assert!(agent.handle.work_session_face_due().await.is_some());

    // The folder does not show the work session key: nothing is due, and
    // a minute passes without a redraw.
    f.mock.tap(&f.device, FOLDER);
    let back = KeyFace::new(FaceStyle::Back, "Back");
    let shown = f.wait_face(&back, None);
    assert_eq!(agent.handle.work_session_face_due().await, None);
    f.jump.by(MINUTE);
    idle_for(500).await;
    assert_eq!(f.writes(), shown);

    // Back: the key shows the time at once, and ticks again.
    f.mock.tap(&f.device, KEY);
    f.wait_face(&working("0:01"), None);
    assert!(agent.handle.work_session_face_due().await.is_some());

    // Unplugged: nothing is due.
    f.mock.unplug(&f.device);
    f.wait_connected(&agent, false).await;
    assert_eq!(agent.handle.work_session_face_due().await, None);
    f.jump.by(MINUTE);
    idle_for(300).await;
    assert_eq!(agent.handle.work_session_face_due().await, None);

    // Plugged in again: the current time, and the next minute is due.
    f.mock.plug(&models::stream_deck_mk2(), "FACES-1");
    f.wait_connected(&agent, true).await;
    f.wait_face(&working("0:02"), None);
    assert!(agent.handle.work_session_face_due().await.is_some());
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seconds_are_opt_in_and_redraw_every_second() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    f.configure(&agent, |s| s.elapsed_format = ElapsedFormat::Seconds)
        .await;
    op(&agent, Op::Start, Gesture::Click).await;
    wait_state(&agent, S::Working).await;
    // The key counts up by itself, once per second.
    let first = f.wait_face(&working("0:00:01"), None);
    let left = agent.handle.work_session_face_due().await.unwrap() - f.jump.now_ms();
    assert!((0..=1000).contains(&left), "{left} ms");
    f.wait_face(&working("0:00:02"), Some(first + 1));
    f.wait_face(&working("0:00:03"), Some(first + 2));

    // Back to minutes: the face changes at once, and then it waits for
    // the minute.
    f.configure(&agent, |s| s.elapsed_format = ElapsedFormat::Minutes)
        .await;
    let shown = f.wait_face(&working("0:00"), None);
    idle_for(1500).await;
    assert_eq!(f.writes(), shown);
    agent.shutdown().await;
}
