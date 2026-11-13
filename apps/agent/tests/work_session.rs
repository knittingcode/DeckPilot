//! The work session service of the agent (docs/gestures-and-work-session.md
//! §5–§7, Phase 9): operations as key gestures ask for them (through the
//! `test-hooks` entry point until bindings drive them), tagged start and
//! verification runs, recovery after restarts, sleep and clock jumps, the
//! heartbeat, and a session that outlives devices and profile changes.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use deckpilot_agent::service::work_session::RunPurpose;
use deckpilot_agent::{
    Clock, Ignored, PressContext, WorkSessionOperation as Op, WorkSessionOutcome,
};
use deckpilot_core::gesture::Gesture;
use deckpilot_core::graph::{ValueKind, WorkflowOutput};
use deckpilot_core::work_session::{
    VerificationOutcome, WorkSessionProblem, WorkSessionSnapshot, WorkSessionState as S,
};
use deckpilot_core::{Profile, VerificationId, WorkSessionId, Workflow, WorkflowId};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, DeviceId, models};
use deckpilot_ipc::{AgentPaths, KeyMaterial, Request};
use deckpilot_storage::DatabaseKey;
use deckpilot_storage::work_sessions::{
    GapKind, NewWorkSession, SessionStatus, VerificationResult, WorkSessionSettings,
};
use serde_json::json;
use support::{TIMEOUT, TempDir, TestAgent, data_dir, graph, graph_workflow, waiting_workflow};

/// The wall clock of the agent, moved by `offset` milliseconds so tests can
/// make it jump while monotonic time goes on.
///
/// Between jumps it advances with the monotonic clock (`Instant`), not with
/// the system clock: the agent measures a jump as the wall delta minus the
/// monotonic delta, and the system clock does not keep step with `Instant`
/// exactly (time synchronisation corrects it, notably on Windows runners),
/// which would add or remove a few milliseconds of its own.
#[derive(Clone)]
struct Jump {
    base: SystemTime,
    start: Instant,
    offset: Arc<AtomicI64>,
}

impl Default for Jump {
    fn default() -> Self {
        Self {
            base: SystemTime::now(),
            start: Instant::now(),
            offset: Arc::default(),
        }
    }
}

impl Jump {
    fn now(&self) -> SystemTime {
        let ms = self.offset.load(Ordering::SeqCst);
        let shift = Duration::from_millis(ms.unsigned_abs());
        let now = self.base + self.start.elapsed();
        if ms >= 0 { now + shift } else { now - shift }
    }

    fn clock(&self) -> Clock {
        let jump = self.clone();
        Arc::new(move || jump.now())
    }

    fn by(&self, ms: i64) {
        self.offset.fetch_add(ms, Ordering::SeqCst);
    }

    fn now_ms(&self) -> i64 {
        i64::try_from(self.now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap()
    }
}

const MINUTE: i64 = 60_000;

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
        let dir = TempDir::new("deckpilot-work-session");
        let mut profile = Profile::new("Work");
        profile.workflows = workflows;
        let (paths, key) = data_dir(&dir, &profile, &[]);
        let mock = MockBackend::new();
        let device = mock.plug(&models::stream_deck_mk2(), "WORK-0001");
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
        TestAgent::start_unlocked(&self.paths, &self.key, &self.mock, Some(self.jump.clock())).await
    }

    fn settings(&self) -> WorkSessionSettings {
        WorkSessionSettings::defaults(self.profile.id)
    }

    /// Saves the session settings of the profile and has the agent reload.
    async fn configure(&self, agent: &TestAgent, change: impl FnOnce(&mut WorkSessionSettings)) {
        let store = agent.store();
        let mut settings = store.work_session_settings(self.profile.id).unwrap();
        change(&mut settings);
        store
            .save_work_session_settings(&settings, self.jump.now_ms())
            .unwrap();
        agent.request(Request::ReloadConfiguration).await.unwrap();
    }
}

/// `start → [wait] → out`: sets `canFinish` (and `details`) from templates.
fn verification(name: &str, can_finish: Option<&str>, details: &str, wait_ms: u32) -> Workflow {
    let mut values = serde_json::Map::new();
    if let Some(can_finish) = can_finish {
        values.insert(
            "canFinish".into(),
            json!({"type": "template", "template": can_finish}),
        );
    }
    values.insert(
        "details".into(),
        json!({"type": "template", "template": details}),
    );
    let mut nodes = vec![json!({"id": "start", "type": "trigger"})];
    let mut edges = Vec::new();
    let mut last = "start";
    if wait_ms > 0 {
        nodes.push(json!({"id": "wait", "type": "delay", "ms": wait_ms}));
        edges.push(json!({"from": "start", "to": "wait"}));
        last = "wait";
    }
    nodes.push(json!({"id": "out", "type": "output", "values": values}));
    edges.push(json!({"from": last, "to": "out"}));
    let mut workflow = graph_workflow(name, graph(&json!(nodes), &json!(edges)));
    workflow.outputs = vec![
        WorkflowOutput::new("canFinish", ValueKind::Boolean),
        WorkflowOutput::new("details", ValueKind::Text),
    ];
    workflow
}

/// A workflow whose run fails: its output is not a number.
fn failing(name: &str) -> Workflow {
    let mut workflow = graph_workflow(
        name,
        graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "out", "type": "output", "values": {
                    "count": {"type": "template", "template": "many"}
                }},
            ]),
            &json!([{"from": "start", "to": "out"}]),
        ),
    );
    workflow.outputs = vec![WorkflowOutput::new("count", ValueKind::Integer)];
    workflow
}

async fn op(agent: &TestAgent, operation: Op, gesture: Gesture) -> WorkSessionOutcome {
    agent
        .handle
        .work_session_operation(operation, gesture, None)
        .await
        .expect("the agent answers")
}

async fn press(
    agent: &TestAgent,
    operation: Op,
    gesture: Gesture,
    press: &PressContext,
) -> WorkSessionOutcome {
    agent
        .handle
        .work_session_operation(operation, gesture, Some(press.clone()))
        .await
        .expect("the agent answers")
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

fn has_notice(snapshot: &WorkSessionSnapshot, problem: WorkSessionProblem) -> bool {
    snapshot.notices.iter().any(|n| n.problem == problem)
}

/// Starts a session without a start workflow and long-presses it to
/// `readyToFinish` (no verification workflow, D12).
async fn ready_without_verification(agent: &TestAgent) -> WorkSessionSnapshot {
    if snapshot(agent).state == S::Idle {
        assert_eq!(
            op(agent, Op::Start, Gesture::Click).await,
            WorkSessionOutcome::Applied
        );
    }
    wait_state(agent, S::Working).await;
    assert_eq!(
        op(agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    wait_state(agent, S::ReadyToFinish).await
}

/// After the arming delay.
fn armed(snapshot: &WorkSessionSnapshot) -> PressContext {
    PressContext::of(snapshot, Instant::now())
}

async fn wait_armed() {
    tokio::time::sleep(
        deckpilot_agent::service::work_session::ARMING_DELAY + Duration::from_millis(100),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_verification_a_long_press_approves_and_one_click_finishes_once() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    assert_eq!(snapshot(&agent).state, S::Idle);

    // T1 + T2 at once without a start workflow; the timer runs.
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let working = wait_state(&agent, S::Working).await;
    let session = working.session.unwrap();
    assert!(working.started_at_ms.is_some());
    // A second start while one is active does nothing.
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Ignored(Ignored::NotNow)
    );

    // D12: a click in working never leads towards finishing.
    assert_eq!(
        op(&agent, Op::Verify, Gesture::Click).await,
        WorkSessionOutcome::Ignored(Ignored::HoldToFinish)
    );
    assert_eq!(Ignored::HoldToFinish.hint(), "Hold to finish");
    assert_eq!(snapshot(&agent).state, S::Working);
    // Finishing is not possible from working at all.
    assert_eq!(
        op(&agent, Op::Finish, Gesture::Click).await,
        WorkSessionOutcome::Ignored(Ignored::StateChanged)
    );

    let ready = ready_without_verification(&agent).await;
    assert_eq!(ready.session, Some(session));
    let verification = ready.verification.clone().unwrap();
    assert_eq!(
        verification.outcome,
        Some(VerificationOutcome {
            can_finish: true,
            details: Some("no verification configured".into())
        })
    );
    assert_eq!(ready.approval, Some(verification.id));

    // The arming delay: a click right after entering ready is ignored.
    assert_eq!(
        op(&agent, Op::Finish, Gesture::Click).await,
        WorkSessionOutcome::Ignored(Ignored::NotArmed)
    );
    wait_armed().await;
    // Only a single click finishes.
    for gesture in [Gesture::DoubleClick, Gesture::LongPress] {
        assert_eq!(
            press(&agent, Op::Finish, gesture, &armed(&ready)).await,
            WorkSessionOutcome::Ignored(Ignored::NotAClick)
        );
    }
    // Two clicks with the same approval at once: exactly one finishes.
    let click = armed(&ready);
    let (first, second) = tokio::join!(
        press(&agent, Op::Finish, Gesture::Click, &click),
        press(&agent, Op::Finish, Gesture::Click, &click),
    );
    let mut outcomes = [first, second];
    outcomes.sort_by_key(|o| format!("{o:?}"));
    assert_eq!(
        outcomes,
        [WorkSessionOutcome::AlreadyDone, WorkSessionOutcome::Applied]
    );
    let idle = wait_state(&agent, S::Idle).await;
    assert_eq!(idle.session, None);
    // A duplicated signal later still answers "already finished".
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &click).await,
        WorkSessionOutcome::AlreadyDone
    );

    let store = agent.store();
    let record = store.work_session(session).unwrap().unwrap();
    assert_eq!(record.status, SessionStatus::Stopped);
    let intervals = store.work_session_intervals(session).unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, record.stopped_at_ms);
    let events: Vec<String> = store
        .work_session_transitions(session)
        .unwrap()
        .into_iter()
        .map(|t| t.event)
        .collect();
    assert_eq!(
        events,
        [
            "start",
            "startSucceeded",
            "verify",
            "verificationPassed",
            "finish",
            "stopCompleted"
        ]
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_start_workflow_runs_first_and_its_failure_leaves_idle_with_the_error() {
    let fast = waiting_workflow("Prepare", 300);
    let slow = waiting_workflow("Prepare slowly", 30_000);
    let broken = failing("Broken start");
    let (fast_id, slow_id, broken_id) = (fast.id, slow.id, broken.id);
    let f = Fixture::new(vec![fast, slow, broken]);
    let agent = f.agent().await;

    // A failing start workflow: idle, with its error, and no time counted.
    f.configure(&agent, |s| s.start_workflow = Some(broken_id))
        .await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let idle = wait_for(&agent, "the failed start", |s| {
        s.state == S::Idle && has_notice(s, WorkSessionProblem::StartFailed)
    })
    .await;
    assert!(
        idle.notices[0].message.contains("start workflow failed"),
        "{idle:#?}"
    );
    let runs = agent.wait_for_runs(broken_id, |r| r.len() == 1).await;
    assert_eq!(
        runs[0].trigger,
        deckpilot_core::ExecutionTrigger::WorkSession
    );

    // A start workflow that cannot run (it no longer exists): the same.
    f.configure(&agent, |s| s.start_workflow = Some(WorkflowId::new()))
        .await;
    assert!(matches!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Failed(_)
    ));
    assert_eq!(snapshot(&agent).state, S::Idle);
    assert!(has_notice(
        &snapshot(&agent),
        WorkSessionProblem::StartFailed
    ));

    // Cancelled while it runs: idle, nothing counted.
    f.configure(&agent, |s| s.start_workflow = Some(slow_id))
        .await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let starting = wait_state(&agent, S::Starting).await;
    let cancelled = starting.session.unwrap();
    assert_eq!(
        op(&agent, Op::CancelStart, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    let idle = wait_for(&agent, "the cancelled start", |s| s.state == S::Idle).await;
    assert!(idle.notices[0].message.contains("cancelled"), "{idle:#?}");
    let store = agent.store();
    assert_eq!(
        store.work_session(cancelled).unwrap().unwrap().status,
        SessionStatus::Aborted
    );
    assert_eq!(store.work_session_intervals(cancelled).unwrap().len(), 0);

    // Success: working, and the time of the start workflow is not counted
    // (D10).
    f.configure(&agent, |s| s.start_workflow = Some(fast_id))
        .await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    assert_eq!(snapshot(&agent).state, S::Starting);
    let working = wait_state(&agent, S::Working).await;
    let record = store
        .work_session(working.session.unwrap())
        .unwrap()
        .unwrap();
    assert!(record.started_at_ms.unwrap() - record.requested_at_ms >= 300);
    assert!(record.start_execution.is_some());
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_results_are_typed_and_never_stop_the_timer() {
    let no = verification("Not yet", Some("false"), "2 tests fail", 0);
    let yes = verification("Done", Some("true"), "", 0);
    let missing = verification("Silent", None, "nothing to say", 0);
    let mut text = verification("Wrong type", Some("true"), "", 0);
    text.outputs[0].kind = ValueKind::Text;
    let mut invalid = verification("Invalid", Some("maybe"), "", 0);
    invalid.outputs[0].kind = ValueKind::Boolean;
    let broken = failing("Broken check");
    let ids = [no.id, yes.id, missing.id, text.id, invalid.id, broken.id];
    let f = Fixture::new(vec![no, yes, missing, text, invalid, broken]);
    let [no, yes, missing, text, invalid, broken] = ids;
    let agent = f.agent().await;
    f.configure(&agent, |s| s.verification_workflow = Some(no))
        .await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let working = wait_state(&agent, S::Working).await;
    let session = working.session.unwrap();
    let started = working.started_at_ms;

    // With a verification workflow, the click checks; a long press does not.
    assert_eq!(
        op(&agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Ignored(Ignored::ClickToCheck)
    );

    // canFinish = false: back to working with the details.
    assert_eq!(
        op(&agent, Op::Verify, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let rejected = wait_for(&agent, "the rejection", |s| {
        s.state == S::Working && s.verification.as_ref().is_some_and(|v| v.outcome.is_some())
    })
    .await;
    assert_eq!(
        rejected.verification.clone().unwrap().outcome,
        Some(VerificationOutcome {
            can_finish: false,
            details: Some("2 tests fail".into())
        })
    );
    assert!(has_notice(&rejected, WorkSessionProblem::NotReady));

    // Failures of every kind are failures (T7), never a "no".
    for (workflow, expected) in [
        (text, "boolean"),
        (missing, "did not set `canFinish`"),
        // The run fails: `maybe` is not a boolean.
        (invalid, "The verification failed"),
        (broken, "does not declare the output `canFinish`"),
    ] {
        f.configure(&agent, |s| s.verification_workflow = Some(workflow))
            .await;
        let before = snapshot(&agent).verification.map(|v| v.id);
        let _ = op(&agent, Op::Verify, Gesture::Click).await;
        let failed = wait_for(&agent, expected, |s| {
            s.state == S::Working
                && s.verification.as_ref().map(|v| v.id) != before
                && has_notice(s, WorkSessionProblem::VerificationFailed)
        })
        .await;
        let notice = &failed.notices[0];
        assert!(notice.message.contains(expected), "{expected}: {notice:#?}");
        assert_eq!(failed.verification.unwrap().outcome, None);
    }

    // Repeated verification: now it passes.
    f.configure(&agent, |s| s.verification_workflow = Some(yes))
        .await;
    assert_eq!(
        op(&agent, Op::Verify, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    assert!(ready.notices.is_empty(), "{ready:#?}");

    // The timer ran through all of it: one interval, still open.
    let store = agent.store();
    let intervals = store.work_session_intervals(session).unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, None);
    assert_eq!(ready.started_at_ms, started);
    assert!(
        store
            .work_session_elapsed_ms(session, f.jump.now_ms())
            .unwrap()
            > 0
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_verification_at_a_time_and_cancelling_it_returns_to_working() {
    let slow = verification("Slow check", Some("true"), "", 30_000);
    let slow_id = slow.id;
    let f = Fixture::new(vec![slow]);
    let agent = f.agent().await;
    f.configure(&agent, |s| s.verification_workflow = Some(slow_id))
        .await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let working = wait_state(&agent, S::Working).await;

    // Two presses that both began in working: one verification only.
    let at_press = PressContext::of(&working, Instant::now());
    let (first, second) = tokio::join!(
        press(&agent, Op::Verify, Gesture::Click, &at_press),
        press(&agent, Op::Verify, Gesture::Click, &at_press),
    );
    let mut outcomes = [first, second];
    outcomes.sort_by_key(|o| format!("{o:?}"));
    assert_eq!(
        outcomes,
        [
            WorkSessionOutcome::Applied,
            WorkSessionOutcome::Ignored(Ignored::StateChanged)
        ]
    );
    let checking = wait_state(&agent, S::VerifyingEnd).await;
    // A press in verifyingEnd does not start another one either.
    assert_eq!(
        op(&agent, Op::Verify, Gesture::Click).await,
        WorkSessionOutcome::Ignored(Ignored::NotNow)
    );
    assert_eq!(agent.runs(slow_id).await.len(), 1);

    assert_eq!(
        op(&agent, Op::CancelVerification, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    let back = wait_for(&agent, "the cancelled check", |s| s.state == S::Working).await;
    assert!(has_notice(&back, WorkSessionProblem::VerificationFailed));
    assert!(back.notices[0].message.contains("cancelled"));
    assert_eq!(back.session, checking.session);
    assert_eq!(back.started_at_ms, working.started_at_ms);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_results_and_approvals_never_move_the_current_session() {
    let no = verification("Not yet", Some("false"), "", 0);
    let yes = verification("Done", Some("true"), "", 0);
    let slow = verification("Slow", Some("true"), "", 30_000);
    let (no_id, yes_id, slow_id) = (no.id, yes.id, slow.id);
    let f = Fixture::new(vec![no, yes, slow]);
    let agent = f.agent().await;
    f.configure(&agent, |s| s.verification_workflow = Some(no_id))
        .await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let session = wait_state(&agent, S::Working).await.session.unwrap();
    let _ = op(&agent, Op::Verify, Gesture::Click).await;
    let first = wait_for(&agent, "the first check", |s| {
        s.verification.as_ref().is_some_and(|v| v.outcome.is_some())
    })
    .await
    .verification
    .unwrap()
    .id;

    f.configure(&agent, |s| s.verification_workflow = Some(yes_id))
        .await;
    let _ = op(&agent, Op::Verify, Gesture::Click).await;
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    let second = ready.approval.unwrap();
    assert_ne!(first, second);

    // A late "yes" of the first verification changes nothing.
    let old = RunPurpose::Verify {
        session,
        verification: first,
    };
    agent.handle.work_session_run_ended(old, Some(true)).await;
    agent.handle.work_session_run_ended(old, None).await;
    let still = snapshot(&agent);
    assert_eq!(
        (still.state, still.approval),
        (S::ReadyToFinish, Some(second))
    );

    // A finish click with an older approval is refused.
    wait_armed().await;
    let mut stale = armed(&ready);
    stale.approval = Some(first);
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &stale).await,
        WorkSessionOutcome::Ignored(Ignored::StateChanged)
    );
    assert_eq!(snapshot(&agent).state, S::ReadyToFinish);

    // Changed settings drop the approval (D9): a click with it no longer
    // finishes, and a late result of it is dropped as well.
    f.configure(&agent, |s| s.verification_workflow = Some(slow_id))
        .await;
    let working = wait_state(&agent, S::Working).await;
    assert!(has_notice(
        &working,
        WorkSessionProblem::ApprovalInvalidated
    ));
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&ready)).await,
        WorkSessionOutcome::Ignored(Ignored::StateChanged)
    );
    let _ = op(&agent, Op::Verify, Gesture::Click).await;
    let checking = wait_state(&agent, S::VerifyingEnd).await;
    let current = checking.verification.unwrap().id;
    let late = RunPurpose::Verify {
        session,
        verification: second,
    };
    agent.handle.work_session_run_ended(late, Some(true)).await;
    assert_eq!(snapshot(&agent).state, S::VerifyingEnd);
    // Only the current verification applies.
    let now = RunPurpose::Verify {
        session,
        verification: current,
    };
    agent.handle.work_session_run_ended(now, Some(true)).await;
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    assert_eq!(ready.approval, Some(current));

    // Finish, start a new session: results and clicks of the old one do
    // nothing to it.
    wait_armed().await;
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&ready)).await,
        WorkSessionOutcome::Applied
    );
    wait_state(&agent, S::Idle).await;
    f.configure(&agent, |s| s.verification_workflow = None)
        .await;
    let next = ready_without_verification(&agent).await;
    assert_ne!(next.session, Some(session));
    agent.handle.work_session_run_ended(now, Some(true)).await;
    wait_armed().await;
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&ready)).await,
        WorkSessionOutcome::AlreadyDone
    );
    let unchanged = snapshot(&agent);
    assert_eq!(
        (unchanged.state, unchanged.session),
        (S::ReadyToFinish, next.session)
    );
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restarts_recover_every_persisted_state() {
    let slow_start = waiting_workflow("Prepare slowly", 30_000);
    let slow_check = verification("Slow check", Some("true"), "", 30_000);
    let (start_id, check_id) = (slow_start.id, slow_check.id);
    let f = Fixture::new(vec![slow_start, slow_check]);

    // Starting: the start is interrupted (T3); nothing was counted.
    let agent = f.agent().await;
    f.configure(&agent, |s| s.start_workflow = Some(start_id))
        .await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let starting = wait_state(&agent, S::Starting).await.session.unwrap();
    agent.shutdown().await;
    let agent = f.agent().await;
    let idle = snapshot(&agent);
    assert_eq!(idle.state, S::Idle);
    assert!(has_notice(&idle, WorkSessionProblem::StartInterrupted));
    assert_eq!(
        agent
            .store()
            .work_session(starting)
            .unwrap()
            .unwrap()
            .status,
        SessionStatus::Aborted
    );

    // Working, and the agent was down longer than the review threshold:
    // the time is counted and flagged (D8).
    f.configure(&agent, |s| {
        s.start_workflow = None;
        s.verification_workflow = Some(check_id);
    })
    .await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let session = wait_state(&agent, S::Working).await.session.unwrap();
    agent.shutdown().await;
    f.jump.by(20 * MINUTE);
    let agent = f.agent().await;
    let working = snapshot(&agent);
    assert_eq!(
        (working.state, working.session),
        (S::Working, Some(session))
    );
    assert!(has_notice(&working, WorkSessionProblem::NeedsReview));
    assert!(working.elapsed_ms >= 20 * 60_000);
    let reviews = agent.store().work_session_reviews(session).unwrap();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].kind, GapKind::AgentDown);

    // VerifyingEnd: the verification is interrupted (T7), the timer runs.
    let _ = op(&agent, Op::Verify, Gesture::Click).await;
    wait_state(&agent, S::VerifyingEnd).await;
    agent.shutdown().await;
    let agent = f.agent().await;
    let back = snapshot(&agent);
    assert_eq!(back.state, S::Working);
    assert!(has_notice(
        &back,
        WorkSessionProblem::VerificationInterrupted
    ));

    // ReadyToFinish: the approval belonged to the earlier agent (T9).
    f.configure(&agent, |s| s.verification_workflow = None)
        .await;
    let ready = ready_without_verification(&agent).await;
    agent.shutdown().await;
    let agent = f.agent().await;
    let back = snapshot(&agent);
    assert_eq!((back.state, back.approval), (S::Working, None));
    assert!(has_notice(&back, WorkSessionProblem::ApprovalInvalidated));
    wait_armed().await;
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&ready)).await,
        WorkSessionOutcome::Ignored(Ignored::StateChanged)
    );
    let intervals = agent.store().work_session_intervals(session).unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, None);

    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_cut_short_is_completed_at_the_next_start() {
    // A session left in stopping, as by an agent that died between T10 and
    // T11: the stop time is fixed, and recovery completes the stop.
    let f = Fixture::new(Vec::new());
    let store = support::open_store(&f.paths, &f.key);
    let (id, v, now) = (WorkSessionId::new(), VerificationId::new(), f.jump.now_ms());
    store
        .begin_work_session(&NewWorkSession {
            id,
            profile: f.profile.id,
            project: None,
            start_workflow: None,
            at_ms: now,
        })
        .unwrap();
    store.work_session_start_succeeded(id, now).unwrap();
    store
        .begin_work_session_verification(id, SessionStatus::Working, v, None, now)
        .unwrap();
    let pass = VerificationResult::Outcome(VerificationOutcome {
        can_finish: true,
        details: None,
    });
    store
        .record_work_session_verification(id, v, &pass, "an earlier agent", now)
        .unwrap();
    store
        .finish_work_session(id, v, "an earlier agent", now + 1000)
        .unwrap();
    assert_eq!(
        store.work_session(id).unwrap().unwrap().status,
        SessionStatus::Stopping
    );
    drop(store);
    let agent = f.agent().await;
    assert_eq!(snapshot(&agent).state, S::Idle);
    let record = agent.store().work_session(id).unwrap().unwrap();
    assert_eq!(record.status, SessionStatus::Stopped);
    assert_eq!(record.stopped_at_ms, Some(now + 1000));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approvals_expire_and_changed_settings_invalidate_them() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    f.configure(&agent, |s| s.approval_ttl_seconds = 60).await;
    let ready = ready_without_verification(&agent).await;
    // Past the lifetime (shorter than a sleep): T9 approvalExpired.
    f.jump.by(61_000);
    let expired = wait_for(&agent, "the expiry", |s| s.state == S::Working).await;
    assert!(has_notice(&expired, WorkSessionProblem::ApprovalExpired));
    wait_armed().await;
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&ready)).await,
        WorkSessionOutcome::Ignored(Ignored::StateChanged)
    );

    // Saving the settings unchanged keeps it; a change drops it.
    let ready = ready_without_verification(&agent).await;
    f.configure(&agent, |_| {}).await;
    assert_eq!(snapshot(&agent).state, S::ReadyToFinish);
    f.configure(&agent, |s| s.review_gap_seconds = 600).await;
    let invalidated = wait_state(&agent, S::Working).await;
    assert!(has_notice(
        &invalidated,
        WorkSessionProblem::ApprovalInvalidated
    ));
    assert_ne!(
        f.settings(),
        agent.store().work_session_settings(f.profile.id).unwrap()
    );

    // Continue working (long press in ready) is T9 without a notice.
    let _ = ready_without_verification(&agent).await;
    assert_eq!(
        op(&agent, Op::ContinueWorking, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    let back = wait_state(&agent, S::Working).await;
    assert_eq!(back.session, ready.session);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sleep_and_clock_jumps_are_flagged_and_drop_the_approval() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    let ready = ready_without_verification(&agent).await;
    let session = ready.session.unwrap();

    // The computer "sleeps" 20 minutes: wall time jumps, monotonic time
    // does not. Counted, flagged, and the approval is gone.
    f.jump.by(20 * MINUTE);
    let woke = wait_for(&agent, "the wake-up", |s| {
        s.state == S::Working && has_notice(s, WorkSessionProblem::NeedsReview)
    })
    .await;
    assert!(has_notice(&woke, WorkSessionProblem::ApprovalInvalidated));
    assert!(woke.elapsed_ms >= 20 * 60_000);
    let store = agent.store();
    let reviews = store.work_session_reviews(session).unwrap();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].kind, GapKind::Sleep);
    assert!(reviews[0].to_ms - reviews[0].from_ms >= 20 * MINUTE);

    // The clock is set back 30 minutes: flagged as well.
    f.jump.by(-30 * MINUTE);
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while store.work_session_reviews(session).unwrap().len() < 2 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no review of the clock change"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let reviews = store.work_session_reviews(session).unwrap();
    assert_eq!(reviews[1].kind, GapKind::ClockBack);
    assert!(reviews[1].to_ms < reviews[1].from_ms);
    // Short jumps are neither sleep nor a review.
    f.jump.by(30_000);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(store.work_session_reviews(session).unwrap().len(), 2);
    assert_eq!(snapshot(&agent).state, S::Working);
    assert_eq!(snapshot(&agent).session, Some(session));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_heartbeat_moves_last_alive_once_a_minute_and_counts_nothing() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let session = wait_state(&agent, S::Working).await.session.unwrap();
    let store = agent.store();
    let alive = || {
        store
            .work_session(session)
            .unwrap()
            .unwrap()
            .last_alive_at_ms
    };
    let transitions = || store.work_session_transitions(session).unwrap().len();
    let (first, logged) = (alive(), transitions());
    // Many checks (every 50 ms) within the minute write nothing.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(alive(), first);
    f.jump.by(61_000);
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while alive() < first + 60_000 {
        assert!(tokio::time::Instant::now() < deadline, "no heartbeat");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let beat = alive();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(alive(), beat, "once per minute only");
    assert_eq!(transitions(), logged);
    assert_eq!(store.work_session_intervals(session).unwrap().len(), 1);
    // A clean shutdown writes the last one.
    f.jump.by(5_000);
    agent.shutdown().await;
    assert!(alive() >= beat + 5_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_and_verify_need_the_sessions_profile_finish_does_not() {
    let f = Fixture::new(Vec::new());
    let home = Profile::new("Home");
    let agent = f.agent().await;
    {
        use deckpilot_storage::ConfigStore;
        agent.store().save_profile(&home).unwrap();
    }
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let working = wait_state(&agent, S::Working).await;
    assert_eq!(working.profile, Some(f.profile.id));

    agent
        .request(Request::SetActiveProfile { profile: home.id })
        .await
        .unwrap();
    // The session goes on under another profile, but cannot be verified
    // there, and no second one starts (one per database, D7).
    assert_eq!(snapshot(&agent).state, S::Working);
    assert_eq!(
        op(&agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Ignored(Ignored::ProfileNotActive)
    );
    let idle_press = PressContext {
        state: S::Idle,
        session: None,
        approval: None,
        at: Instant::now(),
    };
    assert_eq!(
        press(&agent, Op::Start, Gesture::Click, &idle_press).await,
        WorkSessionOutcome::Ignored(Ignored::StateChanged)
    );

    agent
        .request(Request::SetActiveProfile {
            profile: f.profile.id,
        })
        .await
        .unwrap();
    let ready = ready_without_verification(&agent).await;
    agent
        .request(Request::SetActiveProfile { profile: home.id })
        .await
        .unwrap();
    wait_armed().await;
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&ready)).await,
        WorkSessionOutcome::Applied
    );
    wait_state(&agent, S::Idle).await;
    agent.shutdown().await;
}

async fn connected(agent: &TestAgent, device: &DeviceId) -> bool {
    agent
        .status()
        .await
        .devices
        .iter()
        .any(|d| &d.id == device && d.state == ConnectionState::Connected)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnects_and_locks_never_stop_the_session() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let working = wait_state(&agent, S::Working).await;

    f.mock.unplug(&f.device);
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while connected(&agent, &f.device).await {
        assert!(tokio::time::Instant::now() < deadline, "still connected");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(snapshot(&agent).state, S::Working);
    f.mock.plug(&models::stream_deck_mk2(), "WORK-0001");
    while !connected(&agent, &f.device).await {
        assert!(tokio::time::Instant::now() < deadline, "not reconnected");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let after = snapshot(&agent);
    assert_eq!((after.state, after.session), (S::Working, working.session));

    // Locked: nothing can be written, so operations do nothing; the
    // session is there again after the unlock.
    agent.request(Request::Lock).await.unwrap();
    assert_eq!(
        op(&agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Ignored(Ignored::Locked)
    );
    agent
        .request(Request::Unlock {
            key: KeyMaterial::new(f.key.to_hex()),
        })
        .await
        .unwrap();
    let unlocked = snapshot(&agent);
    assert_eq!(
        (unlocked.state, unlocked.session),
        (S::Working, working.session)
    );
    assert!(unlocked.notices.is_empty(), "{unlocked:#?}");
    let intervals = agent
        .store()
        .work_session_intervals(working.session.unwrap())
        .unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, None);
    agent.shutdown().await;
}

/// The verification id of the session's current check.
fn current_check(snapshot: &WorkSessionSnapshot) -> VerificationId {
    snapshot.verification.as_ref().expect("a verification").id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_press_in_ready_verifies_again_with_a_fresh_approval() {
    // A real check that passes after 300 ms; a "no" is handed in through
    // the test hook while a run is under way.
    let check = verification("Check", Some("true"), "", 300);
    let check_id = check.id;
    let f = Fixture::new(vec![check]);
    let agent = f.agent().await;
    f.configure(&agent, |s| s.verification_workflow = Some(check_id))
        .await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let session = wait_state(&agent, S::Working).await.session.unwrap();

    // A long press in working still does not check (a click does).
    assert_eq!(
        op(&agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Ignored(Ignored::ClickToCheck)
    );
    assert_eq!(
        op(&agent, Op::Verify, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let ready = wait_state(&agent, S::ReadyToFinish).await;
    let first = ready.approval.expect("approved");

    // In ready, a long press runs the check again and drops the approval.
    assert_eq!(
        op(&agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    let checking = snapshot(&agent);
    assert_eq!(checking.state, S::VerifyingEnd);
    assert_eq!(checking.approval, None);
    assert_ne!(current_check(&checking), first);
    let again = wait_state(&agent, S::ReadyToFinish).await;
    assert_eq!(again.approval, Some(current_check(&checking)));
    assert_eq!(agent.runs(check_id).await.len(), 2);
    // The old approval cannot finish.
    wait_armed().await;
    assert_ne!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&ready)).await,
        WorkSessionOutcome::Applied
    );
    assert_eq!(snapshot(&agent).approval, again.approval);

    // Verifying again can also say no: back to working, timer running,
    // and no approval left to finish with.
    assert_eq!(
        op(&agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    let third = current_check(&snapshot(&agent));
    agent
        .handle
        .work_session_run_ended(
            RunPurpose::Verify {
                session,
                verification: third,
            },
            Some(false),
        )
        .await;
    let working = wait_state(&agent, S::Working).await;
    assert_eq!((working.session, working.approval), (Some(session), None));
    // The real run's late "yes" is not applied.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(snapshot(&agent).state, S::Working);
    assert_ne!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&again)).await,
        WorkSessionOutcome::Applied
    );
    let intervals = agent.store().work_session_intervals(session).unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, None);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_verification_a_long_press_in_ready_approves_afresh() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    let ready = ready_without_verification(&agent).await;
    wait_armed().await;
    // D12 again: a new approval at once, with a new arming delay.
    assert_eq!(
        op(&agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    let fresh = wait_for(&agent, "a fresh approval", |s| {
        s.state == S::ReadyToFinish && s.approval.is_some() && s.approval != ready.approval
    })
    .await;
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&fresh)).await,
        WorkSessionOutcome::Ignored(Ignored::NotArmed)
    );
    wait_armed().await;
    assert_ne!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&ready)).await,
        WorkSessionOutcome::Applied
    );
    assert_eq!(
        press(&agent, Op::Finish, Gesture::Click, &armed(&fresh)).await,
        WorkSessionOutcome::Applied
    );
    wait_state(&agent, S::Idle).await;
    agent.shutdown().await;
}
