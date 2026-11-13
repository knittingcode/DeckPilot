//! The work session over protocol 8 (docs/gestures-and-work-session.md §9):
//! every request against a real agent over its socket, the
//! `workSessionChanged` event, protocol 7 clients that know none of it,
//! and bounded texts. Sessions are operated through the `test-hooks` entry
//! point, as a key would: no request can start, verify or finish one (D11).

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime};

use deckpilot_agent::{Clock, WorkSessionOperation as Op, WorkSessionOutcome};
use deckpilot_core::gesture::Gesture;
use deckpilot_core::graph::{ValueKind, WorkflowInput, WorkflowOutput};
use deckpilot_core::work_session::{
    MAX_DETAILS, WorkSessionProblem, WorkSessionSnapshot, WorkSessionState as S,
};
use deckpilot_core::{Profile, ProfileId, WorkSessionId, Workflow, WorkflowId};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::models;
use deckpilot_ipc::framing::{read_message, write_message};
use deckpilot_ipc::work_session::{
    ElapsedFormat, IntervalEndReason, WorkSessionRecordState, WorkSessionReviewKind,
    WorkSessionReviewResolution, WorkSessionSettings, WorkSessionSummary,
};
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, Capability, ClientError, ClientKind, ErrorCode, Request,
    Response, SharedStream,
};
use deckpilot_storage::DatabaseKey;
use serde_json::json;
use support::{TIMEOUT, TempDir, TestAgent, data_dir, graph, graph_workflow, waiting_workflow};
use tokio::io::BufReader;

const MINUTE: i64 = 60_000;

/// The agent's wall clock, moved by `offset` ms from real time.
#[derive(Clone, Default)]
struct Jump(Arc<AtomicI64>);

impl Jump {
    fn clock(&self) -> Clock {
        let offset = self.0.clone();
        Arc::new(move || {
            let ms = offset.load(Ordering::SeqCst);
            let shift = Duration::from_millis(ms.unsigned_abs());
            if ms >= 0 {
                SystemTime::now() + shift
            } else {
                SystemTime::now() - shift
            }
        })
    }

    fn by(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

struct Fixture {
    _dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
    mock: MockBackend,
    profile: Profile,
    jump: Jump,
}

impl Fixture {
    fn new(workflows: Vec<Workflow>) -> Self {
        let dir = TempDir::new("deckpilot-work-session-ipc");
        let mut profile = Profile::new("Work");
        profile.workflows = workflows;
        let (paths, key) = data_dir(&dir, &profile, &[]);
        let mock = MockBackend::new();
        mock.plug(&models::stream_deck_mk2(), "WORK-IPC-1");
        Self {
            _dir: dir,
            paths,
            key,
            mock,
            profile,
            jump: Jump::default(),
        }
    }

    async fn agent(&self) -> TestAgent {
        TestAgent::start_unlocked(&self.paths, &self.key, &self.mock, Some(self.jump.clock())).await
    }

    fn settings(&self) -> WorkSessionSettings {
        WorkSessionSettings {
            profile: self.profile.id,
            start_workflow: None,
            verification_workflow: None,
            approval_ttl_seconds: 600,
            review_gap_seconds: 900,
            elapsed_format: ElapsedFormat::Minutes,
            updated_at_ms: None,
        }
    }
}

/// `start → out`, setting `canFinish` and `details`.
fn verification(name: &str, can_finish: &str, details: &str, sensitive: bool) -> Workflow {
    let mut workflow = graph_workflow(
        name,
        graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "out", "type": "output", "values": {
                    "canFinish": {"type": "template", "template": can_finish},
                    "details": {"type": "template", "template": details},
                }},
            ]),
            &json!([{"from": "start", "to": "out"}]),
        ),
    );
    let mut details = WorkflowOutput::new("details", ValueKind::Text);
    details.sensitive = sensitive;
    workflow.outputs = vec![
        WorkflowOutput::new("canFinish", ValueKind::Boolean),
        details,
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
    reached.unwrap_or_else(|_| panic!("{what}: not reached"))
}

fn has_notice(snapshot: &WorkSessionSnapshot, problem: WorkSessionProblem) -> bool {
    snapshot.notices.iter().any(|n| n.problem == problem)
}

async fn get(agent: &TestAgent) -> WorkSessionSnapshot {
    match agent.request(Request::GetWorkSession).await.unwrap() {
        Response::WorkSession { snapshot } => *snapshot,
        other => panic!("unexpected {other:?}"),
    }
}

async fn history(agent: &TestAgent, limit: u32, before: Option<i64>) -> Vec<WorkSessionSummary> {
    match agent
        .request(Request::ListWorkSessions { limit, before })
        .await
        .unwrap()
    {
        Response::WorkSessions { entries } => entries,
        other => panic!("unexpected {other:?}"),
    }
}

async fn save(
    agent: &TestAgent,
    settings: WorkSessionSettings,
) -> Result<WorkSessionSettings, ClientError> {
    match agent
        .request(Request::SaveWorkSessionSettings {
            settings: Box::new(settings),
        })
        .await?
    {
        Response::WorkSessionSettings { settings } => Ok(*settings),
        other => panic!("unexpected {other:?}"),
    }
}

async fn resolve(
    agent: &TestAgent,
    session: WorkSessionId,
    review: i64,
    count: bool,
) -> Result<WorkSessionSummary, ClientError> {
    match agent
        .request(Request::ResolveWorkSessionReview {
            session,
            review,
            count,
        })
        .await?
    {
        Response::WorkSessionEntry { entry } => Ok(*entry),
        other => panic!("unexpected {other:?}"),
    }
}

async fn clear(agent: &TestAgent, session: Option<WorkSessionId>) -> WorkSessionSnapshot {
    match agent
        .request(Request::ClearWorkSessionNotices { session })
        .await
        .unwrap()
    {
        Response::WorkSession { snapshot } => *snapshot,
        other => panic!("unexpected {other:?}"),
    }
}

fn code(result: Result<impl std::fmt::Debug, ClientError>) -> ErrorCode {
    match result {
        Err(ClientError::Agent(error)) => error.code,
        other => panic!("expected an agent error, got {other:?}"),
    }
}

async fn next_change(events: &mut AgentClient) -> WorkSessionSnapshot {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("a workSessionChanged in time")
            .unwrap();
        if let AgentEvent::WorkSessionChanged { snapshot } = event {
            return *snapshot;
        }
    }
}

#[allow(clippy::too_many_lines)] // Every refusal, then the save.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_are_checked_saved_and_read_back() {
    let start = waiting_workflow("Prepare", 10);
    let check = verification("Check", "true", "", false);
    let mut needs_input = waiting_workflow("Needs a branch", 10);
    needs_input.inputs = vec![{
        let mut input = WorkflowInput::new("branch", ValueKind::Text);
        input.required = true;
        input
    }];
    let mut fills_itself = waiting_workflow("Session inputs", 10);
    fills_itself.inputs = vec![{
        let mut input = WorkflowInput::new("sessionId", ValueKind::Text);
        input.required = true;
        input
    }];
    let ids = [start.id, check.id, needs_input.id, fills_itself.id];
    let f = Fixture::new(vec![start, check, needs_input, fills_itself]);
    let [start, check, needs_input, fills_itself] = ids;
    let agent = f.agent().await;

    // Defaults of a profile that never saved any.
    let defaults = match agent
        .request(Request::GetWorkSessionSettings {
            profile: f.profile.id,
        })
        .await
        .unwrap()
    {
        Response::WorkSessionSettings { settings } => *settings,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(defaults, f.settings());
    assert_eq!(
        code(
            agent
                .request(Request::GetWorkSessionSettings {
                    profile: ProfileId::new()
                })
                .await
        ),
        ErrorCode::NotFound
    );

    // Refused: an unknown profile or workflow, a start workflow with
    // inputs a session cannot give, a verification workflow without the
    // contract, values out of range.
    let refused = [
        WorkSessionSettings {
            profile: ProfileId::new(),
            ..f.settings()
        },
        WorkSessionSettings {
            start_workflow: Some(WorkflowId::new()),
            ..f.settings()
        },
        WorkSessionSettings {
            start_workflow: Some(needs_input),
            ..f.settings()
        },
        WorkSessionSettings {
            verification_workflow: Some(start),
            ..f.settings()
        },
        WorkSessionSettings {
            approval_ttl_seconds: 59,
            ..f.settings()
        },
        WorkSessionSettings {
            review_gap_seconds: 86_401,
            ..f.settings()
        },
    ];
    let codes = [
        ErrorCode::NotFound,
        ErrorCode::InvalidRequest,
        ErrorCode::InvalidRequest,
        ErrorCode::InvalidRequest,
        ErrorCode::InvalidRequest,
        ErrorCode::InvalidRequest,
    ];
    for (settings, expected) in refused.into_iter().zip(codes) {
        let result = save(&agent, settings.clone()).await;
        assert_eq!(code(result), expected, "{settings:?}");
    }

    // Saved and read back; inputs the session fills are fine.
    let wanted = WorkSessionSettings {
        start_workflow: Some(fills_itself),
        verification_workflow: Some(check),
        approval_ttl_seconds: 120,
        review_gap_seconds: 300,
        elapsed_format: ElapsedFormat::Seconds,
        ..f.settings()
    };
    let saved = save(&agent, wanted.clone()).await.unwrap();
    assert!(saved.updated_at_ms.is_some());
    assert_eq!(
        WorkSessionSettings {
            updated_at_ms: None,
            ..saved.clone()
        },
        wanted
    );
    let stored = agent.store().work_session_settings(f.profile.id).unwrap();
    assert_eq!(stored.verification_workflow, Some(check));
    assert_eq!(stored.approval_ttl_seconds, 120);
    let saved = save(
        &agent,
        WorkSessionSettings {
            start_workflow: Some(start),
            ..wanted
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.start_workflow, Some(start));
    agent.shutdown().await;
}

/// Starts a session and finishes it from the key, without verification
/// workflow (D12).
async fn finish_one(agent: &TestAgent) {
    assert_eq!(
        op(agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    wait_for(agent, "working", |s| s.state == S::Working).await;
    assert_eq!(
        op(agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    wait_for(agent, "ready", |s| s.state == S::ReadyToFinish).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        op(agent, Op::Finish, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    wait_for(agent, "idle", |s| s.state == S::Idle).await;
}

#[allow(clippy::too_many_lines)] // One session, followed through.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_session_and_its_history_are_reported_and_announced() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    let mut events = agent.events().await;
    assert!(events.supports(Capability::WorkSession));

    let idle = get(&agent).await;
    assert_eq!(idle.state, S::Idle);
    assert_eq!(history(&agent, 10, None).await, Vec::new());
    assert_eq!(
        code(
            agent
                .request(Request::ListWorkSessions {
                    limit: 0,
                    before: None
                })
                .await
        ),
        ErrorCode::InvalidRequest
    );

    // A key starts the session; the change is announced.
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let working = loop {
        let snapshot = next_change(&mut events).await;
        if snapshot.state == S::Working {
            break snapshot;
        }
    };
    let session = working.session.unwrap();
    assert_eq!(working.profile, Some(f.profile.id));
    let now = get(&agent).await;
    assert_eq!(now.state, S::Working);
    assert_eq!(now.session, Some(session));
    assert!(now.as_of_ms >= working.as_of_ms);

    // Long press (no verification workflow), then a click after the
    // arming delay finishes it.
    assert_eq!(
        op(&agent, Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    let ready = wait_for(&agent, "ready", |s| s.state == S::ReadyToFinish).await;
    assert!(ready.approval.is_some());
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        op(&agent, Op::Finish, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    wait_for(&agent, "idle", |s| s.state == S::Idle).await;

    // Two more sessions, the last one still active.
    finish_one(&agent).await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    wait_for(&agent, "working", |s| s.state == S::Working).await;
    let entries = history(&agent, 10, None).await;
    assert_eq!(entries.len(), 3);
    let active = &entries[0];
    assert_eq!(active.state, WorkSessionRecordState::Working);
    assert_eq!(active.intervals.len(), 1);
    assert_eq!(active.intervals[0].ended_at_ms, None);
    let first = &entries[2];
    assert_eq!(first.session, session);
    assert_eq!(first.state, WorkSessionRecordState::Stopped);
    assert_eq!(first.profile, f.profile.id);
    assert!(first.stopped_at_ms.is_some());
    assert_eq!(
        first.intervals[0].end_reason,
        Some(IntervalEndReason::Stopped)
    );
    assert!(first.elapsed_ms >= 1000);
    let events_seen: Vec<_> = first.transitions.iter().map(|t| t.to.as_str()).collect();
    assert_eq!(
        events_seen,
        [
            "starting",
            "working",
            "verifyingEnd",
            "readyToFinish",
            "stopping",
            "stopped"
        ]
    );
    let outcome = first.verification.clone().unwrap().outcome.unwrap();
    assert!(outcome.can_finish);
    // Paging: older than the second entry.
    let older = history(&agent, 10, Some(entries[1].requested_at_ms)).await;
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].session, session);
    assert_eq!(history(&agent, 1, None).await.len(), 1);

    // As JSON, the shapes the app reads.
    let json = serde_json::to_value(first).unwrap();
    for field in [
        "session",
        "profile",
        "state",
        "requestedAtMs",
        "startedAtMs",
        "stoppedAtMs",
        "elapsedMs",
        "intervals",
        "reviews",
        "transitions",
        "verification",
    ] {
        assert!(json.get(field).is_some(), "{field}: {json}");
    }
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protocol_7_clients_see_and_ask_nothing_of_it() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    let mut current = agent.events().await;
    let mut old = AgentClient::connect_with_protocol(&agent.paths, ClientKind::Desktop, 7)
        .await
        .unwrap();
    assert!(old.supports(Capability::Variables));
    assert!(!old.supports(Capability::WorkSession));
    old.subscribe().await.unwrap();

    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    assert_eq!(next_change(&mut current).await.state, S::Working);
    // The old client saw none of it.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while let Ok(event) = tokio::time::timeout_at(deadline, old.next_event()).await {
        let event = event.unwrap();
        assert!(event.since_protocol() <= 7, "{event:?}");
    }
    // The client refuses to send the requests ...
    let refused = old.request(Request::GetWorkSession).await;
    assert!(
        matches!(
            refused,
            Err(ClientError::Unsupported {
                needs: 8,
                protocol: 7,
                ..
            })
        ),
        "{refused:?}"
    );
    // ... and the agent refuses them on a protocol 7 connection anyway.
    let stream = SharedStream::new(agent.paths.endpoint().connect().await.unwrap());
    let mut reader = BufReader::new(stream.clone());
    let mut writer = stream;
    let token = std::fs::read_to_string(&agent.paths.token_file).unwrap();
    write_message(
        &mut writer,
        &json!({"type": "hello", "protocol": 7, "token": token.trim(), "client": "desktop"}),
    )
    .await
    .unwrap();
    let welcome: serde_json::Value = read_message(&mut reader).await.unwrap().unwrap();
    assert!(
        !welcome["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("workSession")),
        "{welcome}"
    );
    for (id, request) in [
        json!({"type": "getWorkSession"}),
        json!({"type": "listWorkSessions", "limit": 5}),
        json!({"type": "clearWorkSessionNotices", "session": null}),
    ]
    .into_iter()
    .enumerate()
    {
        write_message(
            &mut writer,
            &json!({"type": "request", "id": id, "request": request}),
        )
        .await
        .unwrap();
        let refused: serde_json::Value = read_message(&mut reader).await.unwrap().unwrap();
        assert_eq!(refused["error"]["code"], "unsupported", "{refused}");
    }
    drop((reader, writer));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flagged_gaps_are_excluded_and_counted_again() {
    let f = Fixture::new(Vec::new());
    let agent = f.agent().await;
    assert_eq!(
        op(&agent, Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let working = wait_for(&agent, "working", |s| s.state == S::Working).await;
    let session = working.session.unwrap();
    let mut events = agent.events().await;

    // The computer "sleeps" 20 minutes: counted and flagged.
    f.jump.by(20 * MINUTE);
    let woke = wait_for(&agent, "the review", |s| {
        has_notice(s, WorkSessionProblem::NeedsReview)
    })
    .await;
    assert!(woke.elapsed_ms >= 20 * 60_000);
    let entry = history(&agent, 1, None).await.remove(0);
    assert_eq!(entry.reviews.len(), 1);
    let review = entry.reviews[0].clone();
    assert_eq!(review.kind, WorkSessionReviewKind::Sleep);
    assert_eq!(review.resolution, None);

    // Only reviews of the session named.
    assert_eq!(
        code(resolve(&agent, WorkSessionId::new(), review.id, false).await),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(resolve(&agent, session, review.id + 100, false).await),
        ErrorCode::NotFound
    );

    // Excluded: the gap no longer counts, and the app hears of it.
    let excluded = resolve(&agent, session, review.id, false).await.unwrap();
    assert_eq!(
        excluded.reviews[0].resolution,
        Some(WorkSessionReviewResolution::Excluded)
    );
    assert!(excluded.reviews[0].resolved_at_ms.is_some());
    assert!(excluded.elapsed_ms < 20 * 60_000, "{excluded:#?}");
    assert!(
        excluded
            .intervals
            .iter()
            .any(|i| i.end_reason == Some(IntervalEndReason::GapExcluded))
    );
    let announced = loop {
        let snapshot = next_change(&mut events).await;
        if !has_notice(&snapshot, WorkSessionProblem::NeedsReview) {
            break snapshot;
        }
    };
    assert!(announced.elapsed_ms < 20 * 60_000);
    assert_eq!(announced.state, S::Working);

    // Counted again: the time is back, and announced although no state
    // changed.
    let counted = resolve(&agent, session, review.id, true).await.unwrap();
    assert_eq!(
        counted.reviews[0].resolution,
        Some(WorkSessionReviewResolution::Counted)
    );
    assert!(counted.elapsed_ms >= 20 * 60_000);
    let announced = next_change(&mut events).await;
    assert!(announced.elapsed_ms >= 20 * 60_000);
    assert_eq!(get(&agent).await.session, Some(session));
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notices_are_cleared_for_a_session_or_when_idle() {
    let broken = failing("Broken start");
    let no = verification("Not yet", "false", "2 tests fail", false);
    let (broken_id, no_id) = (broken.id, no.id);
    let f = Fixture::new(vec![broken, no]);
    let agent = f.agent().await;

    // A failed start leaves idle with its notice; clearing it needs no
    // session.
    save(
        &agent,
        WorkSessionSettings {
            start_workflow: Some(broken_id),
            ..f.settings()
        },
    )
    .await
    .unwrap();
    op(&agent, Op::Start, Gesture::Click).await;
    let idle = wait_for(&agent, "the failed start", |s| {
        s.state == S::Idle && has_notice(s, WorkSessionProblem::StartFailed)
    })
    .await;
    assert_eq!(get(&agent).await.notices, idle.notices);
    let cleared = clear(&agent, None).await;
    assert_eq!(cleared.state, S::Idle);
    assert!(cleared.notices.is_empty(), "{cleared:#?}");
    assert_eq!(get(&agent).await.notices, Vec::new());
    let aborted = history(&agent, 1, None).await.remove(0);
    assert_eq!(aborted.state, WorkSessionRecordState::Aborted);
    assert_eq!(aborted.notice, None);

    // The active session's notice ("not ready").
    save(
        &agent,
        WorkSessionSettings {
            verification_workflow: Some(no_id),
            ..f.settings()
        },
    )
    .await
    .unwrap();
    op(&agent, Op::Start, Gesture::Click).await;
    wait_for(&agent, "working", |s| s.state == S::Working).await;
    op(&agent, Op::Verify, Gesture::Click).await;
    let rejected = wait_for(&agent, "the rejection", |s| {
        has_notice(s, WorkSessionProblem::NotReady)
    })
    .await;
    let session = rejected.session.unwrap();
    assert_eq!(
        code(
            agent
                .request(Request::ClearWorkSessionNotices {
                    session: Some(WorkSessionId::new())
                })
                .await
        ),
        ErrorCode::NotFound
    );
    let cleared = clear(&agent, Some(session)).await;
    assert_eq!(cleared.state, S::Working);
    assert!(cleared.notices.is_empty(), "{cleared:#?}");
    // The verification's result stays.
    assert_eq!(
        cleared
            .verification
            .unwrap()
            .outcome
            .unwrap()
            .details
            .as_deref(),
        Some("2 tests fail")
    );
    assert_eq!(get(&agent).await.notices, Vec::new());
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn details_are_bounded_and_sensitive_ones_never_sent() {
    let long = verification("Chatty", "false", &"x".repeat(MAX_DETAILS * 3), false);
    let secret = verification("Secret", "false", "token 12345", true);
    let (long_id, secret_id) = (long.id, secret.id);
    let f = Fixture::new(vec![long, secret]);
    let agent = f.agent().await;
    let mut events = agent.events().await;
    save(
        &agent,
        WorkSessionSettings {
            verification_workflow: Some(long_id),
            ..f.settings()
        },
    )
    .await
    .unwrap();
    op(&agent, Op::Start, Gesture::Click).await;
    wait_for(&agent, "working", |s| s.state == S::Working).await;
    op(&agent, Op::Verify, Gesture::Click).await;
    wait_for(&agent, "the rejection", |s| {
        has_notice(s, WorkSessionProblem::NotReady)
    })
    .await;
    let details = |snapshot: &WorkSessionSnapshot| {
        snapshot
            .verification
            .clone()
            .and_then(|v| v.outcome)
            .and_then(|o| o.details)
    };
    let sent = get(&agent).await;
    assert_eq!(
        details(&sent).unwrap().chars().count(),
        MAX_DETAILS,
        "{sent:#?}"
    );
    let announced = loop {
        let snapshot = next_change(&mut events).await;
        if has_notice(&snapshot, WorkSessionProblem::NotReady) {
            break snapshot;
        }
    };
    assert_eq!(details(&announced).unwrap().chars().count(), MAX_DETAILS);
    let entry = history(&agent, 1, None).await.remove(0);
    let stored = entry
        .verification
        .unwrap()
        .outcome
        .unwrap()
        .details
        .unwrap();
    assert_eq!(stored.chars().count(), MAX_DETAILS);
    for notice in sent.notices.iter().chain(&announced.notices) {
        assert!(notice.message.chars().count() <= 501, "{notice:?}");
    }

    // Sensitive details are never sent, nor stored.
    save(
        &agent,
        WorkSessionSettings {
            verification_workflow: Some(secret_id),
            ..f.settings()
        },
    )
    .await
    .unwrap();
    let before = sent.verification.map(|v| v.id);
    op(&agent, Op::Verify, Gesture::Click).await;
    wait_for(&agent, "the second rejection", |s| {
        s.verification.as_ref().map(|v| v.id) != before
            && s.verification.as_ref().is_some_and(|v| v.outcome.is_some())
    })
    .await;
    let sent = get(&agent).await;
    assert_eq!(details(&sent), None, "{sent:#?}");
    let all = serde_json::to_string(&history(&agent, 10, None).await).unwrap()
        + &serde_json::to_string(&sent).unwrap();
    assert!(!all.contains("12345"), "{all}");
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_locked_agent_tells_nothing() {
    let f = Fixture::new(Vec::new());
    let agent = TestAgent::start(&f.paths, &f.key, &f.mock, Some(f.jump.clock()));
    for request in [
        Request::GetWorkSession,
        Request::ListWorkSessions {
            limit: 10,
            before: None,
        },
        Request::GetWorkSessionSettings {
            profile: f.profile.id,
        },
        Request::ClearWorkSessionNotices { session: None },
    ] {
        assert_eq!(code(agent.request(request).await), ErrorCode::Locked);
    }
    agent.unlock().await;
    assert_eq!(get(&agent).await.state, S::Idle);
    agent.shutdown().await;
}
