//! The work session end to end (`docs/gestures-and-work-session.md`,
//! Phase 9, workstream T): a real agent with a simulated MK.2 whose key 0
//! is bound to the stateful `workSession` controller, operated by key
//! gestures only, across the gesture engine, the stateful bindings, the
//! work session service and store, the key faces, the
//! `system.work.session.*` variables and protocol 8.
//!
//! Deterministic by construction:
//!
//! - **Wall clock.** The agent's clock is fake: it stands still unless a
//!   test moves it, so elapsed times, faces, gaps and variable values are
//!   exact.
//! - **Gestures.** A click is queued as one batch (press and release share
//!   their read time), so it is never mistaken for a long press. A long
//!   press is held until the agent reports it fired, then released. No
//!   assertion depends on how long anything took.
//! - **"Nothing happened".** Checked after a barrier: a click on a
//!   click-only key, which fires on its press. The device's inputs are
//!   handled in order, so everything sent before it has been handled.
//! - **Guards.** The only real waits are the lower bounds of the 1 s
//!   repeat guard and arming delay (D9), measured from instants the agent
//!   reported; waiting longer cannot change an outcome.
//!
//! Each test runs under the support watchdog with bounded waits.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use deckpilot_agent::service::work_session::{ARMING_DELAY, NO_VERIFICATION};
use deckpilot_agent::{
    Clock, GestureHooks, Ignored, PressContext, WorkSessionOperation as Op, WorkSessionOutcome,
};
use deckpilot_core::gesture::{
    BoundGestures, Gesture, GestureFired, GestureTimings, KeyGestureSettings,
};
use deckpilot_core::graph::{ValueKind, WorkflowOutput};
use deckpilot_core::stateful::StatefulBinding;
use deckpilot_core::variables::{CatalogFilter, PreviewItem, ResolutionContext};
use deckpilot_core::work_session::{
    WorkSessionProblem, WorkSessionSnapshot, WorkSessionState as S,
};
use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, FaceStyle, Indicator, KeyFace,
    PageId, ParameterValue, Profile, Project, ProjectKey, RuntimeOutcome, Trigger, Workflow,
    WorkflowId,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, ControlInput, DeviceId, models};
use deckpilot_ipc::{AgentPaths, Request, Response};
use deckpilot_modules::general;
use deckpilot_storage::DatabaseKey;
use deckpilot_storage::work_sessions::{
    GapKind, HistoryQuery, IntervalEnd, SessionStatus, WorkSessionSettings, WorkSessionSummary,
};
use serde_json::{Value, json};
use support::{TIMEOUT, TempDir, TestAgent, data_dir, graph, graph_workflow};

/// The work session key: click and long press, default rules.
const KEY: ControlId = ControlId::Key { index: 0 };
/// A click-only key: it fires on the press, so it serves as a barrier.
const MARK: ControlId = ControlId::Key { index: 4 };
const LONG_MS: u16 = 400;
const MINUTE: i64 = 60_000;
/// 2026-11-13T08:00:00Z, when every test's clock starts.
const T0: i64 = 1_794_556_800_000;
const T0_TEXT: &str = "2026-11-13T08:00:00Z";
/// The repeat guard of stateful keys and the arming delay are both 1 s.
const GUARD: Duration = ARMING_DELAY;

/// The agent's wall clock: stands at `T0` until a test moves it.
#[derive(Clone)]
struct FakeClock(Arc<AtomicI64>);

impl FakeClock {
    fn new() -> Self {
        Self(Arc::new(AtomicI64::new(T0)))
    }

    fn clock(&self) -> Clock {
        let ms = self.0.clone();
        Arc::new(move || {
            UNIX_EPOCH + Duration::from_millis(ms.load(Ordering::SeqCst).unsigned_abs())
        })
    }

    fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }

    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Every dispatched gesture, with what its binding did.
#[derive(Default)]
struct Recorder(Mutex<Vec<(GestureFired, Did)>>);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Did {
    Stateful,
    Ran(String),
    Other,
}

impl GestureHooks for Recorder {
    fn bindings(
        &self,
        _device: &DeviceId,
        _page: Option<PageId>,
        _control: ControlId,
    ) -> Option<(BoundGestures, GestureTimings)> {
        None
    }

    fn fired(&self, _fired: &GestureFired) {}

    fn dispatched(&self, fired: &GestureFired, outcome: &RuntimeOutcome) {
        let did = match outcome {
            RuntimeOutcome::StatefulRequested { .. } => Did::Stateful,
            RuntimeOutcome::Action { result, .. } => Did::Ran(result.message.clone()),
            _ => Did::Other,
        };
        self.0.lock().unwrap().push((fired.clone(), did));
    }
}

impl Recorder {
    fn seen(&self) -> Vec<(GestureFired, Did)> {
        self.0.lock().unwrap().clone()
    }
}

/// `start → out`: answers `canFinish = true`.
fn check() -> Workflow {
    let mut workflow = graph_workflow(
        "Ready check",
        graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "out", "type": "output", "values": {
                    "canFinish": {"type": "template", "template": "true"}
                }},
            ]),
            &json!([{"from": "start", "to": "out"}]),
        ),
    );
    workflow.outputs = vec![WorkflowOutput::new("canFinish", ValueKind::Boolean)];
    workflow
}

/// The variables a workflow step reads, in this order.
const REPORT: &str = "{{ system.work.session.state }}|{{ system.work.session.elapsedSeconds }}|\
{{ system.work.session.id }}|{{ system.work.session.startedAt }}|{{ system.work.session.projectId }}";

/// Key 0 follows the work session; key 4 logs "mark" on its press. The
/// profile has a project, a `Report` workflow whose step logs the session
/// variables, and `extra` workflows.
fn work_profile(extra: Vec<Workflow>) -> Profile {
    let mut profile = Profile::new("Work");
    profile
        .projects
        .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
    let registry = support::registry();
    let log = |name: &str, message: &str| {
        registry
            .create_instance(
                name,
                general::log_id(),
                &ActionParameters::new().with("message", ParameterValue::Text(message.into())),
            )
            .unwrap()
    };
    let mark = log("Mark", "mark");
    let report = log("Report", "unused");
    let (mark_id, report_id) = (mark.id, report.id);
    profile.actions.extend([mark, report]);
    profile.workflows.push(graph_workflow(
        "Report",
        graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "report", "type": "action", "action": report_id, "mappings": {
                    "message": {"type": "template", "template": REPORT},
                }},
            ]),
            &json!([{"from": "start", "to": "report"}]),
        ),
    ));
    profile.workflows.extend(extra);

    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let page = &mut layout.pages[0];
    let session = || BindingTarget::Stateful(StatefulBinding::work_session());
    page.bindings = vec![
        ControlBinding::new(KEY, Trigger::Click, session()),
        ControlBinding::new(KEY, Trigger::LongPress, session()),
        ControlBinding::new(MARK, Trigger::Click, BindingTarget::action(mark_id)),
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
    clock: FakeClock,
    hooks: Arc<Recorder>,
}

impl Fixture {
    /// A data directory of its own; `verification` becomes the profile's
    /// end check.
    fn new(verification: Option<Workflow>) -> Self {
        let dir = TempDir::new("deckpilot-work-session-e2e");
        let check_id = verification.as_ref().map(|w| w.id);
        let profile = work_profile(verification.into_iter().collect());
        let (paths, key) = data_dir(&dir, &profile, &[]);
        let clock = FakeClock::new();
        let store = support::open_store(&paths, &key);
        let mut settings = WorkSessionSettings::defaults(profile.id);
        settings.verification_workflow = check_id;
        store
            .save_work_session_settings(&settings, clock.now_ms())
            .unwrap();
        drop(store);
        let mock = MockBackend::new();
        let device = mock.plug(&models::stream_deck_mk2(), "E2E-0001");
        Self {
            _dir: dir,
            paths,
            key,
            mock,
            device,
            profile,
            clock,
            hooks: Arc::default(),
        }
    }

    /// A running, unlocked agent with the MK.2 connected and the project
    /// selected.
    async fn agent(&self) -> TestAgent {
        let agent = TestAgent::start_with_gestures(
            &self.paths,
            &self.key,
            &self.mock,
            Some(self.clock.clock()),
            Some(self.hooks.clone()),
        );
        agent.unlock().await;
        self.wait_connected(&agent, true).await;
        agent
            .request(Request::SelectProject {
                project: self.profile.projects[0].id,
            })
            .await
            .unwrap();
        agent
    }

    async fn wait_connected(&self, agent: &TestAgent, wanted: bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let connected = agent
                .status()
                .await
                .devices
                .iter()
                .any(|d| d.id == self.device && d.state == ConnectionState::Connected);
            if connected == wanted {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the MK.2 connection did not change"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn unplug(&self) {
        self.mock.unplug(&self.device);
    }

    fn plug(&self) {
        self.mock.plug(&models::stream_deck_mk2(), "E2E-0001");
    }

    /// Waits until `count` gestures were dispatched; returns the last.
    async fn wait_dispatched(&self, count: usize) -> (GestureFired, Did) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let seen = self.hooks.seen();
            if seen.len() >= count {
                assert_eq!(seen.len(), count, "more gestures than sent: {seen:#?}");
                return seen[count - 1].clone();
            }
            assert!(Instant::now() < deadline, "no gesture {count}: {seen:#?}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// A click on the key, press and release read together; returns the
    /// gesture once the session service has had it.
    async fn click(&self) -> GestureFired {
        let count = self.hooks.seen().len();
        self.mock.send_inputs(
            &self.device,
            &[(KEY, ControlInput::Press), (KEY, ControlInput::Release)],
        );
        let (fired, did) = self.wait_dispatched(count + 1).await;
        assert_eq!(
            (fired.control, fired.gesture, did),
            (KEY, Gesture::Click, Did::Stateful)
        );
        fired
    }

    /// Holds the key until its long press fired, then releases it; the
    /// release is handled before this returns.
    async fn long_press(&self) -> GestureFired {
        let count = self.hooks.seen().len();
        self.mock.send_input(&self.device, KEY, ControlInput::Press);
        let (fired, did) = self.wait_dispatched(count + 1).await;
        assert_eq!(
            (fired.control, fired.gesture, did),
            (KEY, Gesture::LongPress, Did::Stateful)
        );
        assert_eq!(
            fired.at - fired.pressed_at,
            Duration::from_millis(LONG_MS.into()),
            "the key's own long-press time"
        );
        self.mock
            .send_input(&self.device, KEY, ControlInput::Release);
        self.barrier().await;
        fired
    }

    /// Returns once every input sent so far has been handled.
    async fn barrier(&self) {
        let count = self.hooks.seen().len();
        self.mock.send_inputs(
            &self.device,
            &[(MARK, ControlInput::Press), (MARK, ControlInput::Release)],
        );
        let (fired, did) = self.wait_dispatched(count + 1).await;
        assert_eq!((fired.control, did), (MARK, Did::Ran("mark".into())));
    }

    /// Waits out the repeat guard of the key's last operation and, for a
    /// session that is ready by now, the arming delay: both started before
    /// this call (the agent reported the gesture or the state first).
    async fn past_guards(&self) {
        tokio::time::sleep(GUARD + Duration::from_millis(50)).await;
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

    fn history(&self, agent: &TestAgent) -> Vec<WorkSessionSummary> {
        agent
            .store()
            .work_session_history(
                HistoryQuery {
                    profile: None,
                    before_ms: None,
                    limit: 10,
                },
                self.clock.now_ms(),
            )
            .unwrap()
    }
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

/// The session as protocol 8 reports it.
async fn reported(agent: &TestAgent) -> WorkSessionSnapshot {
    match agent.request(Request::GetWorkSession).await.unwrap() {
        Response::WorkSession { snapshot } => *snapshot,
        other => panic!("unexpected {other:?}"),
    }
}

fn face(title: &str, detail: Option<&str>, indicator: Option<Indicator>) -> KeyFace {
    KeyFace {
        detail: detail.map(str::to_owned),
        indicator,
        ..KeyFace::new(FaceStyle::Action, title)
    }
}

fn idle_face() -> KeyFace {
    face("Start Work", None, None)
}

fn working_face(elapsed: &str, indicator: Indicator) -> KeyFace {
    face(elapsed, Some("Working"), Some(indicator))
}

fn ready_face(elapsed: &str) -> KeyFace {
    face("Finish Work", Some(elapsed), Some(Indicator::Ok))
}

/// The states the store's audit log went through, oldest first.
fn audit(agent: &TestAgent, snapshot: &WorkSessionSnapshot) -> Vec<String> {
    agent
        .store()
        .work_session_transitions(snapshot.session.unwrap())
        .unwrap()
        .into_iter()
        .map(|t| t.to)
        .collect()
}

async fn runs(agent: &TestAgent, workflow: WorkflowId) -> usize {
    agent.runs(workflow).await.len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_key_runs_a_session_through_its_verification_workflow() {
    let check = check();
    let check_id = check.id;
    let f = Fixture::new(Some(check));
    let agent = f.agent().await;
    f.wait_face(&idle_face());

    // Idle: a click starts; time counts from now, and the key shows it.
    f.click().await;
    let working = wait_state(&agent, S::Working).await;
    assert_eq!(working.started_at_ms, Some(T0));
    f.wait_face(&working_face("0:00", Indicator::Ok));
    f.clock.advance(5 * MINUTE);
    f.wait_face(&working_face("0:05", Indicator::Ok));

    // Working with a verification workflow: a click runs it.
    f.past_guards().await;
    f.click().await;
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    assert_eq!(ready.session, working.session);
    assert_eq!(runs(&agent, check_id).await, 1);
    f.wait_face(&ready_face("0:05"));

    // Ready: a long press verifies again (T8): the workflow runs once
    // more and gives a fresh approval; the hold never finishes.
    f.past_guards().await;
    f.long_press().await;
    let fresh = wait_for(&agent, "a fresh approval", |s| {
        s.state == S::ReadyToFinish && s.approval.is_some() && s.approval != ready.approval
    })
    .await;
    assert_eq!(fresh.session, working.session);
    assert_eq!(runs(&agent, check_id).await, 2);
    assert_eq!(snapshot(&agent).state, S::ReadyToFinish);

    // Only a click finishes: the same press as a long press is refused.
    f.past_guards().await;
    let held = PressContext::of(&fresh, Instant::now());
    assert_eq!(
        agent
            .handle
            .work_session_operation(Op::Finish, Gesture::LongPress, Some(held))
            .await,
        Some(WorkSessionOutcome::Ignored(Ignored::NotAClick))
    );
    f.click().await;
    let idle = wait_state(&agent, S::Idle).await;
    assert_eq!(idle.session, None);
    f.wait_face(&idle_face());

    let history = f.history(&agent);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].session.status, SessionStatus::Stopped);
    assert_eq!(history[0].session.stopped_at_ms, Some(T0 + 5 * MINUTE));
    assert_eq!(history[0].elapsed_ms, 5 * 60_000);
    let intervals = agent
        .store()
        .work_session_intervals(working.session.unwrap())
        .unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].end_reason, Some(IntervalEnd::Stopped));
    assert_eq!(
        audit(&agent, &working),
        [
            "starting",
            "working",
            "verifyingEnd",
            "readyToFinish",
            "verifyingEnd",
            "readyToFinish",
            "stopping",
            "stopped"
        ]
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_verification_workflow_a_long_press_passes_at_once() {
    let f = Fixture::new(None);
    let agent = f.agent().await;

    // A long press in idle has no rule, and its release is no click.
    f.long_press().await;
    assert_eq!(snapshot(&agent).state, S::Idle);
    assert_eq!(f.history(&agent), []);

    f.click().await;
    let working = wait_state(&agent, S::Working).await;
    // D12: a click in working only says how to finish.
    f.past_guards().await;
    f.click().await;
    f.wait_face(&face("0:00", Some("Hold to finish"), Some(Indicator::Ok)));
    f.barrier().await;
    assert_eq!(snapshot(&agent).state, S::Working);

    // A long press passes at once: no workflow runs.
    f.past_guards().await;
    f.long_press().await;
    let ready = snapshot(&agent);
    assert_eq!(ready.state, S::ReadyToFinish, "{ready:#?}");
    let outcome = ready
        .verification
        .as_ref()
        .unwrap()
        .outcome
        .as_ref()
        .unwrap();
    assert!(outcome.can_finish);
    assert_eq!(outcome.details.as_deref(), Some(NO_VERIFICATION));

    // Ready: a long press approves again at once, with a new approval.
    f.past_guards().await;
    f.long_press().await;
    let fresh = snapshot(&agent);
    assert_eq!(
        (fresh.state, fresh.session),
        (S::ReadyToFinish, working.session)
    );
    assert!(fresh.approval.is_some() && fresh.approval != ready.approval);

    f.past_guards().await;
    f.click().await;
    wait_state(&agent, S::Idle).await;
    assert_eq!(f.history(&agent)[0].session.status, SessionStatus::Stopped);
    // No workflow ran at any point.
    match agent
        .request(Request::ListExecutions {
            workflow: None,
            limit: None,
        })
        .await
        .unwrap()
    {
        Response::Executions { executions } => assert!(executions.is_empty(), "{executions:#?}"),
        other => panic!("unexpected {other:?}"),
    }
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_resumes_the_session_and_flags_only_long_gaps() {
    let f = Fixture::new(None);
    let agent = f.agent().await;
    f.click().await;
    let working = wait_state(&agent, S::Working).await;
    f.clock.advance(3 * MINUTE);
    agent.shutdown().await;

    // Down for 5 minutes: resumed from the store, counted, not flagged.
    f.clock.advance(5 * MINUTE);
    let agent = f.agent().await;
    let resumed = snapshot(&agent);
    assert_eq!(
        (resumed.state, resumed.session),
        (S::Working, working.session)
    );
    assert_eq!(resumed.elapsed_ms, 8 * 60_000);
    assert!(resumed.notices.is_empty(), "{resumed:#?}");
    assert_eq!(
        agent
            .store()
            .work_session_reviews(working.session.unwrap())
            .unwrap(),
        []
    );
    f.wait_face(&working_face("0:08", Indicator::Ok));
    agent.shutdown().await;

    // Down for 20 minutes (over the 15-minute threshold): still counted,
    // and flagged for review.
    f.clock.advance(20 * MINUTE);
    let agent = f.agent().await;
    let resumed = snapshot(&agent);
    assert_eq!(
        (resumed.state, resumed.session),
        (S::Working, working.session)
    );
    assert_eq!(resumed.elapsed_ms, 28 * 60_000);
    assert!(
        resumed
            .notices
            .iter()
            .any(|n| n.problem == WorkSessionProblem::NeedsReview),
        "{resumed:#?}"
    );
    let reviews = agent
        .store()
        .work_session_reviews(working.session.unwrap())
        .unwrap();
    assert_eq!(reviews.len(), 1, "{reviews:#?}");
    assert_eq!(reviews[0].kind, GapKind::AgentDown);
    assert_eq!(
        (reviews[0].from_ms, reviews[0].to_ms),
        (T0 + 8 * MINUTE, T0 + 28 * MINUTE)
    );
    assert_eq!(reviews[0].resolution, None);
    f.wait_face(&working_face("0:28", Indicator::Warning));
    // Protocol 8 tells the same, and the history counts the open review.
    let over_ipc = reported(&agent).await;
    assert_eq!(over_ipc.elapsed_ms, 28 * 60_000);
    assert_eq!(over_ipc.session, working.session);
    let summary = &f.history(&agent)[0];
    assert_eq!((summary.elapsed_ms, summary.open_reviews), (28 * 60_000, 1));
    let elapsed = previews(&agent, &f.profile, &["system.work.session.elapsedSeconds"]).await;
    assert_eq!(elapsed[0]["value"]["value"], 28 * 60);

    // Excluding the gap (the Work Session page's history) takes it off
    // the snapshot, the variable and the key at once.
    let decided = match agent
        .request(Request::ResolveWorkSessionReview {
            session: working.session.unwrap(),
            review: reviews[0].id,
            count: false,
        })
        .await
        .unwrap()
    {
        Response::WorkSessionEntry { entry } => *entry,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(decided.elapsed_ms, 8 * 60_000);
    assert_eq!(decided.intervals.len(), 2, "split around the gap");
    wait_for(&agent, "the gap excluded", |s| s.elapsed_ms == 8 * 60_000).await;
    let elapsed = previews(&agent, &f.profile, &["system.work.session.elapsedSeconds"]).await;
    assert_eq!(elapsed[0]["value"]["value"], 8 * 60);
    f.wait_face(&working_face("0:08", Indicator::Warning));

    // The key goes on from where it was.
    f.long_press().await;
    assert_eq!(snapshot(&agent).state, S::ReadyToFinish);
    f.past_guards().await;
    f.click().await;
    wait_state(&agent, S::Idle).await;
    assert_eq!(f.history(&agent)[0].elapsed_ms, 8 * 60_000);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnect_never_stops_the_session_and_the_face_comes_back() {
    let f = Fixture::new(None);
    let agent = f.agent().await;
    f.click().await;
    let working = wait_state(&agent, S::Working).await;
    let session = working.session.unwrap();

    // Unplugged while working: the timer runs on.
    f.unplug();
    f.wait_connected(&agent, false).await;
    f.clock.advance(2 * MINUTE);
    let now = reported(&agent).await;
    assert_eq!((now.state, now.session), (S::Working, Some(session)));
    assert_eq!(now.elapsed_ms, 2 * 60_000);
    let intervals = agent.store().work_session_intervals(session).unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, None);

    // Plugged in again: the face shows the current time.
    f.plug();
    f.wait_connected(&agent, true).await;
    f.wait_face(&working_face("0:02", Indicator::Ok));

    // Unplugged while ready: the approval stays.
    f.past_guards().await;
    f.long_press().await;
    let ready = snapshot(&agent);
    assert_eq!(ready.state, S::ReadyToFinish);
    f.unplug();
    f.wait_connected(&agent, false).await;
    f.clock.advance(MINUTE);
    let now = reported(&agent).await;
    assert_eq!(
        (now.state, now.session, now.approval),
        (S::ReadyToFinish, Some(session), ready.approval)
    );
    f.plug();
    f.wait_connected(&agent, true).await;
    f.wait_face(&ready_face("0:03"));

    // The key finishes it, with all the time counted.
    f.past_guards().await;
    f.click().await;
    wait_state(&agent, S::Idle).await;
    let history = f.history(&agent);
    assert_eq!(history[0].session.status, SessionStatus::Stopped);
    assert_eq!(history[0].elapsed_ms, 3 * 60_000);
    agent.shutdown().await;
}

/// Previews of `paths` over IPC: each preview's `status` object.
async fn previews(agent: &TestAgent, profile: &Profile, paths: &[&str]) -> Vec<Value> {
    let items = paths
        .iter()
        .map(|reference| PreviewItem {
            reference: (*reference).to_owned(),
            expected: None,
        })
        .collect();
    let response = agent
        .request(Request::PreviewReferences {
            context: Box::new(ResolutionContext::preview(profile.id)),
            items,
        })
        .await
        .unwrap();
    let Response::ReferencePreviews { previews } = response else {
        panic!("previews expected, got {response:?}");
    };
    serde_json::to_value(previews)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["status"].clone())
        .collect()
}

const PATHS: [&str; 6] = [
    "system.work.session.id",
    "system.work.session.state",
    "system.work.session.startedAt",
    "system.work.session.elapsedSeconds",
    "system.work.session.projectId",
    "system.work.session.canFinish",
];

/// Runs `Report`: what its step logged, or why it failed.
async fn report(agent: &TestAgent, profile: &Profile) -> Result<String, String> {
    let execution = agent
        .start_workflow(profile.workflows[0].id)
        .await
        .unwrap()
        .id;
    let record = agent.wait_finished(execution).await;
    let node = record
        .nodes
        .iter()
        .find(|n| n.node.as_str() == "report")
        .unwrap_or_else(|| panic!("no report step in {record:#?}"));
    match &node.error {
        None => Ok(node.message.clone()),
        Some(error) => Err(format!("{}: {}", error.code, error.message)),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn variables_and_workflow_steps_see_the_session_the_key_runs() {
    let f = Fixture::new(None);
    let agent = f.agent().await;
    let project = f.profile.projects[0].id;

    // Listed while idle, without values; a step that needs them fails.
    let listed = match agent
        .request(Request::ListVariables {
            context: Box::new(ResolutionContext::preview(f.profile.id)),
            filter: Box::new(CatalogFilter {
                query: "work session".into(),
                ..CatalogFilter::default()
            }),
        })
        .await
        .unwrap()
    {
        Response::Variables { entries } => entries
            .iter()
            .map(|e| e.reference.to_string())
            .collect::<Vec<_>>(),
        other => panic!("unexpected {other:?}"),
    };
    // The legacy paths (deprecated aliases), then the module's own ids.
    assert_eq!(listed[..6], PATHS);
    assert!(listed[6..].iter().all(|id| id.starts_with("workSession.")));
    assert_eq!(listed.len(), 12);
    for value in previews(&agent, &f.profile, &PATHS).await {
        assert_eq!(value["status"], "missing", "{value}");
    }
    let error = report(&agent, &f.profile).await.unwrap_err();
    assert!(error.starts_with("reference-missing"), "{error}");

    // Working, two minutes in: every value matches the session.
    f.click().await;
    wait_state(&agent, S::Working).await;
    f.clock.advance(2 * MINUTE);
    let session = reported(&agent).await;
    assert_eq!(session.elapsed_ms, 2 * 60_000);
    assert_eq!(session.project, Some(project));
    let id = session.session.unwrap().to_string();
    let values = previews(&agent, &f.profile, &PATHS).await;
    assert_eq!(values[0]["value"], json!({"type": "string", "value": id}));
    assert_eq!(
        values[1]["value"],
        json!({"type": "string", "value": "working"})
    );
    assert_eq!(
        values[2]["value"],
        json!({"type": "string", "value": T0_TEXT})
    );
    assert_eq!(values[3]["value"], json!({"type": "integer", "value": 120}));
    assert_eq!(
        values[4]["value"],
        json!({"type": "projectId", "value": project.to_string()})
    );
    assert_eq!(values[5]["status"], "missing", "not verified yet");
    assert_eq!(
        report(&agent, &f.profile).await.unwrap(),
        format!("working|120|{id}|{T0_TEXT}|{project}")
    );

    // Ready, three minutes in: the state and canFinish follow.
    f.past_guards().await;
    f.long_press().await;
    f.clock.advance(MINUTE);
    let values = previews(&agent, &f.profile, &PATHS).await;
    assert_eq!(values[1]["value"]["value"], "readyToFinish");
    assert_eq!(values[3]["value"]["value"], 180);
    assert_eq!(
        values[5]["value"],
        json!({"type": "boolean", "value": true})
    );
    assert_eq!(
        report(&agent, &f.profile).await.unwrap(),
        format!("readyToFinish|180|{id}|{T0_TEXT}|{project}")
    );

    // Finished: no values again.
    f.past_guards().await;
    f.click().await;
    wait_state(&agent, S::Idle).await;
    for value in previews(&agent, &f.profile, &PATHS).await {
        assert_eq!(value["status"], "missing", "{value}");
    }
    agent.shutdown().await;
}
