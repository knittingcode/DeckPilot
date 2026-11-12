//! Stateful keys end to end (`docs/gestures-and-work-session.md`, section
//! 4): a simulated MK.2 whose key 0 follows the work session with the
//! default rules, operated only by key gestures. The service's own guards
//! are tested in `work_session.rs`; these tests check that the key picks
//! the operation from the state at the press, hands it to the service on
//! the service task, and shows the service's hints.
//!
//! Real time with margins: the key's long press takes 300 ms, and the
//! arming delay and the repeat guard are 1 s each.

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deckpilot_agent::GestureHooks;
use deckpilot_core::gesture::{BoundGestures, GestureFired, GestureTimings, KeyGestureSettings};
use deckpilot_core::graph::{ValueKind, WorkflowOutput};
use deckpilot_core::stateful::StatefulBinding;
use deckpilot_core::work_session::{
    WorkSessionProblem, WorkSessionSnapshot, WorkSessionState as S,
};
use deckpilot_core::{
    BindingTarget, ControlBinding, DeviceLayout, FaceStyle, KeyFace, PageId, Profile,
    RuntimeOutcome, Trigger, Workflow, WorkflowId,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, models};
use deckpilot_ipc::{AgentPaths, Request};
use deckpilot_storage::work_sessions::{HistoryQuery, SessionStatus, WorkSessionSettings};
use deckpilot_storage::{ConfigStore, DatabaseKey};
use serde_json::json;
use support::{TIMEOUT, TempDir, TestAgent, data_dir, graph, graph_workflow, waiting_workflow};

const KEY: ControlId = ControlId::Key { index: 0 };
const LONG_MS: u16 = 300;
/// Longer than the arming delay and the repeat guard.
const SETTLE: Duration = Duration::from_millis(1200);

/// Records the thread every stateful gesture was dispatched on.
#[derive(Default)]
struct Threads(Mutex<Vec<String>>);

impl GestureHooks for Threads {
    fn bindings(
        &self,
        _device: &DeviceId,
        _page: Option<PageId>,
        _control: ControlId,
    ) -> Option<(BoundGestures, GestureTimings)> {
        None
    }

    fn fired(&self, _fired: &GestureFired) {}

    fn dispatched(&self, _fired: &GestureFired, outcome: &RuntimeOutcome) {
        if matches!(outcome, RuntimeOutcome::StatefulRequested { .. }) {
            let name = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_owned();
            self.0.lock().unwrap().push(name);
        }
    }
}

/// `start → [wait] → out`: sets `canFinish` from a template.
fn verification(name: &str, can_finish: &str, wait_ms: u32) -> Workflow {
    let mut nodes = vec![json!({"id": "start", "type": "trigger"})];
    let mut edges = Vec::new();
    let mut last = "start";
    if wait_ms > 0 {
        nodes.push(json!({"id": "wait", "type": "delay", "ms": wait_ms}));
        edges.push(json!({"from": "start", "to": "wait"}));
        last = "wait";
    }
    nodes.push(json!({"id": "out", "type": "output", "values": {
        "canFinish": {"type": "template", "template": can_finish}
    }}));
    edges.push(json!({"from": last, "to": "out"}));
    let mut workflow = graph_workflow(name, graph(&json!(nodes), &json!(edges)));
    workflow.outputs = vec![WorkflowOutput::new("canFinish", ValueKind::Boolean)];
    workflow
}

/// The profile: key 0 follows the work session with the default rules on
/// click and long press.
fn work_profile(workflows: Vec<Workflow>) -> Profile {
    let mut profile = Profile::new("Work");
    profile.workflows = workflows;
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let page = &mut layout.pages[0];
    let session = || BindingTarget::Stateful(StatefulBinding::work_session());
    page.bindings = vec![
        ControlBinding::new(KEY, Trigger::Click, session()),
        ControlBinding::new(KEY, Trigger::LongPress, session()),
    ];
    page.gestures = vec![KeyGestureSettings {
        control: KEY,
        double_click_ms: None,
        long_press_ms: Some(LONG_MS),
    }];
    profile.layouts.push(layout);
    profile
}

struct Fixture {
    _dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
    mock: MockBackend,
    device: DeviceId,
    profile: Profile,
    threads: Arc<Threads>,
}

impl Fixture {
    fn new(workflows: Vec<Workflow>) -> Self {
        let dir = TempDir::new("deckpilot-stateful-keys");
        let profile = work_profile(workflows);
        let (paths, key) = data_dir(&dir, &profile, &[]);
        let mock = MockBackend::new();
        let device = mock.plug(&models::stream_deck_mk2(), "STATEFUL-1");
        Self {
            _dir: dir,
            paths,
            key,
            mock,
            device,
            profile,
            threads: Arc::default(),
        }
    }

    async fn agent(&self) -> TestAgent {
        let agent = TestAgent::start_with_gestures(
            &self.paths,
            &self.key,
            &self.mock,
            None,
            Some(self.threads.clone()),
        );
        agent.unlock().await;
        self.wait_connected(&agent).await;
        agent
    }

    async fn wait_connected(&self, agent: &TestAgent) {
        let deadline = Instant::now() + TIMEOUT;
        while !connected(agent, &self.device).await {
            assert!(Instant::now() < deadline, "the MK.2 did not connect");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn configure(&self, agent: &TestAgent, change: impl FnOnce(&mut WorkSessionSettings)) {
        let store = agent.store();
        let mut settings = store.work_session_settings(self.profile.id).unwrap();
        change(&mut settings);
        store
            .save_work_session_settings(&settings, now_ms())
            .unwrap();
        agent.request(Request::ReloadConfiguration).await.unwrap();
    }

    fn click(&self) {
        self.mock.tap(&self.device, KEY);
    }

    async fn long_press(&self) {
        self.mock.send_input(&self.device, KEY, ControlInput::Press);
        tokio::time::sleep(Duration::from_millis(u64::from(LONG_MS) + 300)).await;
        self.mock
            .send_input(&self.device, KEY, ControlInput::Release);
    }

    /// Waits until key 0 shows `face`.
    fn wait_face(&self, face: &KeyFace) {
        let spec = models::stream_deck_mk2()
            .capabilities
            .keys
            .and_then(|keys| keys.image)
            .unwrap();
        let image = deckpilot_agent::render::render(face, spec.width, spec.height);
        assert!(
            self.mock.wait_for(&self.device, TIMEOUT, |frames, _| {
                frames.get(&KEY) == Some(&image)
            }),
            "key 0 does not show {face:?}"
        );
    }

    fn dispatch_threads(&self) -> Vec<String> {
        self.threads.0.lock().unwrap().clone()
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

async fn connected(agent: &TestAgent, device: &DeviceId) -> bool {
    agent
        .status()
        .await
        .devices
        .iter()
        .any(|d| &d.id == device && d.state == ConnectionState::Connected)
}

fn snapshot(agent: &TestAgent) -> WorkSessionSnapshot {
    agent.handle.work_session().borrow().clone()
}

async fn wait_for(
    agent: &TestAgent,
    what: &str,
    predicate: impl Fn(&WorkSessionSnapshot) -> bool,
) -> WorkSessionSnapshot {
    let mut updates = agent.handle.work_session();
    let reached = tokio::time::timeout(TIMEOUT, async {
        loop {
            {
                let current = updates.borrow_and_update();
                if predicate(&current) {
                    return current.clone();
                }
            }
            updates.changed().await.expect("the agent runs");
        }
    })
    .await;
    reached.unwrap_or_else(|_| panic!("{what}: not reached, {:#?}", snapshot(agent)))
}

async fn wait_state(agent: &TestAgent, state: S) -> WorkSessionSnapshot {
    wait_for(agent, state.name(), |s| s.state == state).await
}

/// Lets a gesture that should do nothing have its chance, then checks
/// that the session is still `state` (and the same session).
async fn stays(agent: &TestAgent, before: &WorkSessionSnapshot) {
    tokio::time::sleep(Duration::from_millis(600)).await;
    let now = snapshot(agent);
    assert_eq!(
        (now.state, now.session, now.approval),
        (before.state, before.session, before.approval),
        "{now:#?}"
    );
}

fn plain_face() -> KeyFace {
    KeyFace::new(FaceStyle::Action, "Work Session")
}

fn hint_face(hint: &str) -> KeyFace {
    plain_face().with_detail(hint)
}

fn sessions(agent: &TestAgent) -> Vec<deckpilot_storage::work_sessions::WorkSessionSummary> {
    agent
        .store()
        .work_session_history(
            HistoryQuery {
                profile: None,
                before_ms: None,
                limit: 100,
            },
            now_ms(),
        )
        .unwrap()
}

async fn runs(agent: &TestAgent, workflow: WorkflowId) -> usize {
    agent.runs(workflow).await.len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_key_starts_verifies_and_finishes_a_session() {
    // The check takes 1.2 s, so a click right after it passed is past the
    // repeat guard but inside the arming delay.
    let check = verification("Check", "true", 1200);
    let check_id = check.id;
    let f = Fixture::new(vec![check]);
    let agent = f.agent().await;
    f.configure(&agent, |s| s.verification_workflow = Some(check_id))
        .await;
    f.wait_face(&plain_face());

    // Idle: a click starts.
    f.click();
    let working = wait_state(&agent, S::Working).await;
    tokio::time::sleep(SETTLE).await;
    // Working with a verification workflow: a click verifies.
    f.click();
    wait_state(&agent, S::VerifyingEnd).await;
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    assert_eq!(ready.session, working.session);
    // Within the arming delay: ignored, still ready.
    f.click();
    stays(&agent, &ready).await;
    tokio::time::sleep(SETTLE).await;
    // A long press checks again and gives a fresh approval.
    f.long_press().await;
    wait_state(&agent, S::VerifyingEnd).await;
    let again = wait_state(&agent, S::ReadyToFinish).await;
    assert_eq!(again.session, working.session);
    assert!(again.approval.is_some() && again.approval != ready.approval);
    tokio::time::sleep(SETTLE).await;
    // A click finishes.
    f.click();
    let idle = wait_state(&agent, S::Idle).await;
    assert_eq!(idle.session, None);
    let record = agent
        .store()
        .work_session(working.session.unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.status, SessionStatus::Stopped);
    assert!(record.stopped_at_ms.is_some());
    assert_eq!(runs(&agent, check_id).await, 2);

    // Every operation ran on the agent's service task, never on a device
    // thread.
    let threads = f.dispatch_threads();
    assert_eq!(threads.len(), 5, "{threads:?}");
    assert!(
        threads.iter().all(|name| !name.starts_with("device")),
        "{threads:?}"
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_verification_keeps_working_and_the_timer() {
    let check = verification("Check", "false", 0);
    let check_id = check.id;
    let f = Fixture::new(vec![check]);
    let agent = f.agent().await;
    f.configure(&agent, |s| s.verification_workflow = Some(check_id))
        .await;

    f.click();
    let working = wait_state(&agent, S::Working).await;
    tokio::time::sleep(SETTLE).await;
    f.click();
    let back = wait_for(&agent, "rejected", |s| {
        s.state == S::Working
            && s.notices
                .iter()
                .any(|n| n.problem == WorkSessionProblem::NotReady)
    })
    .await;
    assert_eq!(back.session, working.session);
    assert_eq!(runs(&agent, check_id).await, 1);
    // The timer runs on; nothing was stopped.
    let intervals = agent
        .store()
        .work_session_intervals(working.session.unwrap())
        .unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, None);
    // A long press in working with a verification workflow does not
    // verify; the key says how.
    tokio::time::sleep(SETTLE).await;
    f.long_press().await;
    f.wait_face(&hint_face("Click to check"));
    stays(&agent, &back).await;
    assert_eq!(runs(&agent, check_id).await, 1);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_verification_a_click_hints_and_a_long_press_approves() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;

    // A long press in idle has no rule: it starts nothing, and its
    // release is not a click.
    f.long_press().await;
    stays(&agent, &snapshot(&agent)).await;
    assert_eq!(snapshot(&agent).state, S::Idle);

    f.click();
    let working = wait_state(&agent, S::Working).await;
    tokio::time::sleep(SETTLE).await;
    // D12: a click does nothing but say how.
    f.click();
    f.wait_face(&hint_face("Hold to finish"));
    stays(&agent, &working).await;
    // A long press approves at once; its release does not click, so it
    // does not finish.
    f.long_press().await;
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    // A click within 1 s of the approval is ignored.
    f.click();
    stays(&agent, &ready).await;
    // The hint is gone with the state it belonged to.
    f.wait_face(&plain_face());
    // A long press in ready approves again at once (D12): a fresh
    // approval with its own arming delay.
    tokio::time::sleep(SETTLE).await;
    f.long_press().await;
    let fresh = wait_for(&agent, "a fresh approval", |s| {
        s.state == S::ReadyToFinish && s.approval.is_some() && s.approval != ready.approval
    })
    .await;
    assert_eq!(fresh.session, working.session);
    tokio::time::sleep(SETTLE).await;
    f.click();
    wait_state(&agent, S::Idle).await;
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_clicks_neither_repeat_nor_chain_operations() {
    let check = verification("Check", "true", 800);
    let check_id = check.id;
    let f = Fixture::new(vec![check]);
    let agent = f.agent().await;
    f.configure(&agent, |s| s.verification_workflow = Some(check_id))
        .await;

    // A double click on Start starts once and does not verify at once.
    f.click();
    f.click();
    let working = wait_state(&agent, S::Working).await;
    stays(&agent, &working).await;
    assert_eq!(runs(&agent, check_id).await, 0);

    // Clicks while checking are ignored; the check runs once and the
    // timer keeps running.
    tokio::time::sleep(SETTLE).await;
    f.click();
    let checking = wait_state(&agent, S::VerifyingEnd).await;
    f.click();
    f.click();
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    assert_eq!(ready.session, checking.session);
    assert_eq!(runs(&agent, check_id).await, 1);

    // A double click on Finish finishes once and does not start the next
    // session.
    tokio::time::sleep(SETTLE).await;
    f.click();
    f.click();
    let idle = wait_state(&agent, S::Idle).await;
    stays(&agent, &idle).await;
    let history = sessions(&agent);
    assert_eq!(history.len(), 1, "{history:#?}");
    assert_eq!(history[0].session.status, SessionStatus::Stopped);
    assert_eq!(Some(history[0].session.id), working.session);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_profile_change_drops_the_press_and_keeps_the_session() {
    let f = Fixture::new(Vec::new());
    let home = Profile::new("Home");
    let agent = f.agent().await;
    agent.store().save_profile(&home).unwrap();

    f.click();
    let working = wait_state(&agent, S::Working).await;
    tokio::time::sleep(SETTLE).await;
    // Pressed, then the profile changes before the long press: the hold
    // and its release do nothing.
    f.mock.send_input(&f.device, KEY, ControlInput::Press);
    agent
        .request(Request::SetActiveProfile { profile: home.id })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(u64::from(LONG_MS) + 300)).await;
    f.mock.send_input(&f.device, KEY, ControlInput::Release);
    stays(&agent, &working).await;
    // The other profile does not bind the key.
    f.long_press().await;
    stays(&agent, &working).await;

    agent
        .request(Request::SetActiveProfile {
            profile: f.profile.id,
        })
        .await
        .unwrap();
    f.wait_face(&plain_face());
    f.long_press().await;
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    assert_eq!(ready.session, working.session);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnect_drops_the_press_and_keeps_the_session() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    f.click();
    let working = wait_state(&agent, S::Working).await;
    tokio::time::sleep(SETTLE).await;

    // Unplugged during a hold: no long press, the session goes on.
    f.mock.send_input(&f.device, KEY, ControlInput::Press);
    tokio::time::sleep(Duration::from_millis(100)).await;
    f.mock.unplug(&f.device);
    let deadline = Instant::now() + TIMEOUT;
    while connected(&agent, &f.device).await {
        assert!(Instant::now() < deadline, "still connected");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(u64::from(LONG_MS) + 300)).await;
    stays(&agent, &working).await;

    f.mock.plug(&models::stream_deck_mk2(), "STATEFUL-1");
    f.wait_connected(&agent).await;
    f.wait_face(&plain_face());
    assert_eq!(snapshot(&agent).session, working.session);
    f.long_press().await;
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    assert_eq!(ready.session, working.session);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_a_restart_in_each_state_the_key_does_the_right_thing() {
    let slow_start = waiting_workflow("Prepare slowly", 30_000);
    let slow_check = verification("Slow check", "true", 30_000);
    let (start_id, check_id) = (slow_start.id, slow_check.id);
    let f = Fixture::new(vec![slow_start, slow_check]);

    // Starting → idle (interrupted): a click starts again, and a long
    // press cancels that start.
    let agent = f.agent().await;
    f.configure(&agent, |s| s.start_workflow = Some(start_id))
        .await;
    f.click();
    wait_state(&agent, S::Starting).await;
    agent.shutdown().await;
    let agent = f.agent().await;
    assert_eq!(snapshot(&agent).state, S::Idle);
    f.click();
    wait_state(&agent, S::Starting).await;
    tokio::time::sleep(SETTLE).await;
    f.long_press().await;
    let idle = wait_state(&agent, S::Idle).await;
    assert_eq!(idle.session, None);

    // Working → working: a click verifies.
    f.configure(&agent, |s| {
        s.start_workflow = None;
        s.verification_workflow = Some(check_id);
    })
    .await;
    tokio::time::sleep(SETTLE).await;
    f.click();
    let working = wait_state(&agent, S::Working).await;
    agent.shutdown().await;
    let agent = f.agent().await;
    assert_eq!(snapshot(&agent).session, working.session);
    f.click();
    wait_state(&agent, S::VerifyingEnd).await;

    // VerifyingEnd → working (interrupted): a click verifies again, and a
    // long press cancels that check.
    agent.shutdown().await;
    let agent = f.agent().await;
    let back = snapshot(&agent);
    assert_eq!((back.state, back.session), (S::Working, working.session));
    f.click();
    wait_state(&agent, S::VerifyingEnd).await;
    tokio::time::sleep(SETTLE).await;
    f.long_press().await;
    wait_state(&agent, S::Working).await;

    // ReadyToFinish → working (the approval was the earlier agent's): a click
    // only hints, a long press approves again, then a click finishes.
    f.configure(&agent, |s| s.verification_workflow = None)
        .await;
    tokio::time::sleep(SETTLE).await;
    f.long_press().await;
    wait_state(&agent, S::ReadyToFinish).await;
    agent.shutdown().await;
    let agent = f.agent().await;
    let back = snapshot(&agent);
    assert_eq!((back.state, back.approval), (S::Working, None));
    f.click();
    f.wait_face(&hint_face("Hold to finish"));
    stays(&agent, &back).await;
    f.long_press().await;
    wait_state(&agent, S::ReadyToFinish).await;
    tokio::time::sleep(SETTLE).await;
    f.click();
    wait_state(&agent, S::Idle).await;
    let record = agent
        .store()
        .work_session(working.session.unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.status, SessionStatus::Stopped);
    agent.shutdown().await;
}
