use deckpilot_core::work_session::{
    TRANSITIONS, VerificationOutcome, WorkSessionNotice, WorkSessionProblem,
};
use deckpilot_core::{
    ExecutionId, Profile, ProfileId, Project, ProjectKey, VerificationId, WorkSessionId, WorkflowId,
};

use super::*;
use crate::{ConfigChanges, ConfigStore, Encryption, SqliteStore, StorageError};

const MIN: i64 = 60_000;
const BOOT: &str = "boot-1";

fn store_with_profile() -> (SqliteStore, Profile) {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut profile = Profile::new("Work");
    profile
        .projects
        .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
    store.save_profile(&profile).unwrap();
    (store, profile)
}

fn new_session(profile: &Profile, at_ms: i64) -> NewWorkSession {
    NewWorkSession {
        id: WorkSessionId::new(),
        profile: profile.id,
        project: profile.projects.first().map(|p| p.id),
        start_workflow: None,
        at_ms,
    }
}

fn notice(problem: WorkSessionProblem, at_ms: i64) -> WorkSessionNotice {
    WorkSessionNotice {
        problem,
        message: "safe text".into(),
        at_ms,
    }
}

fn outcome(can_finish: bool, details: Option<&str>) -> VerificationResult {
    VerificationResult::Outcome(VerificationOutcome {
        can_finish,
        details: details.map(str::to_owned),
    })
}

#[track_caller]
fn applied(transition: Transition) -> WorkSessionRecord {
    match transition {
        Transition::Applied(record) => record,
        other => panic!("not applied: {other:?}"),
    }
}

/// A session in working since `at_ms`.
fn working(store: &SqliteStore, profile: &Profile, at_ms: i64) -> WorkSessionId {
    let new = new_session(profile, at_ms - 1000);
    applied(store.begin_work_session(&new).unwrap());
    applied(store.work_session_start_succeeded(new.id, at_ms).unwrap());
    new.id
}

/// Verifies with a pass at `at_ms`; returns the approval.
fn approve(store: &SqliteStore, id: WorkSessionId, at_ms: i64) -> VerificationId {
    let status = store.work_session(id).unwrap().unwrap().status;
    let v = VerificationId::new();
    applied(
        store
            .begin_work_session_verification(id, status, v, None, at_ms - 10)
            .unwrap(),
    );
    applied(
        store
            .record_work_session_verification(id, v, &outcome(true, None), BOOT, at_ms)
            .unwrap(),
    );
    v
}

/// The whole row and its children, to show that nothing changed.
fn everything(store: &SqliteStore, id: WorkSessionId) -> String {
    format!(
        "{:?}{:?}{:?}{:?}",
        store.work_session(id).unwrap(),
        store.work_session_intervals(id).unwrap(),
        store.work_session_reviews(id).unwrap(),
        store.work_session_transitions(id).unwrap()
    )
}

#[test]
fn a_full_cycle_counts_time_from_working_to_the_stop() {
    let (store, profile) = store_with_profile();
    let workflow = WorkflowId::new();
    let new = NewWorkSession {
        start_workflow: Some(workflow),
        ..new_session(&profile, 1_000)
    };
    let started = applied(store.begin_work_session(&new).unwrap());
    assert_eq!(started.status, SessionStatus::Starting);
    assert_eq!(started.start_workflow, Some(workflow));
    assert_eq!(store.active_work_session().unwrap(), Some(started));
    // The start workflow's duration is not counted (D10).
    assert_eq!(store.work_session_elapsed_ms(new.id, 50_000).unwrap(), 0);
    let execution = ExecutionId::new();
    assert!(
        store
            .set_work_session_start_execution(new.id, execution)
            .unwrap()
    );

    let record = applied(store.work_session_start_succeeded(new.id, 10_000).unwrap());
    assert_eq!(record.started_at_ms, Some(10_000));
    assert_eq!(record.start_execution, Some(execution));
    assert_eq!(
        store
            .work_session_elapsed_ms(new.id, 10_000 + 5 * MIN)
            .unwrap(),
        5 * 60_000
    );

    let v = approve(&store, new.id, 10_000 + 30 * MIN);
    let ready = store.work_session(new.id).unwrap().unwrap();
    assert_eq!(ready.status, SessionStatus::ReadyToFinish);
    assert_eq!(ready.approval, Some(v));
    assert_eq!(ready.can_finish, Some(true));

    let stop = 10_000 + 31 * MIN;
    let stopping = applied(store.finish_work_session(new.id, v, BOOT, stop).unwrap());
    assert_eq!(stopping.status, SessionStatus::Stopping);
    assert_eq!(stopping.stopped_at_ms, Some(stop));
    let stopped = applied(store.complete_work_session_stop(new.id, stop + 5).unwrap());
    assert_eq!(stopped.status, SessionStatus::Stopped);
    assert_eq!(store.active_work_session().unwrap(), None);

    // The time stays fixed at the stop.
    assert_eq!(
        store
            .work_session_elapsed_ms(new.id, stop + 600 * MIN)
            .unwrap(),
        31 * 60_000
    );
    let intervals = store.work_session_intervals(new.id).unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, Some(stop));
    assert_eq!(intervals[0].end_reason, Some(IntervalEnd::Stopped));

    let log: Vec<_> = store
        .work_session_transitions(new.id)
        .unwrap()
        .into_iter()
        .map(|t| format!("{} -{}-> {}", t.from, t.event, t.to))
        .collect();
    assert_eq!(
        log,
        [
            "idle -start-> starting",
            "starting -startSucceeded-> working",
            "working -verify-> verifyingEnd",
            "verifyingEnd -verificationPassed-> readyToFinish",
            "readyToFinish -finish-> stopping",
            "stopping -stopCompleted-> stopped",
        ]
    );

    let history = store
        .work_session_history(
            HistoryQuery {
                profile: Some(profile.id),
                before_ms: None,
                limit: 10,
            },
            stop + 600 * MIN,
        )
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].elapsed_ms, 31 * 60_000);
    assert_eq!(history[0].session.status, SessionStatus::Stopped);
}

#[test]
fn every_logged_transition_is_in_the_core_table() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 1_000);
    let v = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::Working, v, None, 2_000)
        .unwrap();
    store
        .record_work_session_verification(id, v, &outcome(false, None), BOOT, 3_000)
        .unwrap();
    let v = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::Working, v, None, 4_000)
        .unwrap();
    store
        .record_work_session_verification(
            id,
            v,
            &VerificationResult::Failed(notice(WorkSessionProblem::VerificationFailed, 5_000)),
            BOOT,
            5_000,
        )
        .unwrap();
    let v = approve(&store, id, 7_000);
    store
        .invalidate_work_session_approval(id, v, None, 8_000)
        .unwrap();
    let v = approve(&store, id, 9_000);
    let w = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::ReadyToFinish, w, None, 9_500)
        .unwrap();
    assert_eq!(
        store
            .record_work_session_verification(id, v, &outcome(true, None), BOOT, 9_600)
            .unwrap(),
        Transition::Refused(Refusal::StaleVerification)
    );
    let name = |state: deckpilot_core::work_session::WorkSessionState| state.name().to_owned();
    for logged in store.work_session_transitions(id).unwrap() {
        assert!(
            TRANSITIONS.iter().any(|(from, event, to)| {
                name(*from) == logged.from
                    && event_name(*event) == logged.event
                    && name(*to) == logged.to
            }),
            "{logged:?}"
        );
    }
    assert_eq!(store.work_session_transitions(id).unwrap().len(), 12);
}

#[test]
fn only_one_session_is_active() {
    let (store, profile) = store_with_profile();
    let first = new_session(&profile, 1_000);
    applied(store.begin_work_session(&first).unwrap());
    let second = new_session(&profile, 2_000);
    assert_eq!(
        store.begin_work_session(&second).unwrap(),
        Transition::Refused(Refusal::AnotherSessionActive { active: first.id })
    );
    assert_eq!(store.work_session(second.id).unwrap(), None);
    // Repeating the same request is recognised.
    assert!(matches!(
        store.begin_work_session(&first).unwrap(),
        Transition::Repeated(_)
    ));

    // The constraint holds even for a writer that skips the API.
    let raw = store.connection_for_tests();
    let inserted = raw.execute(
        "INSERT INTO work_sessions (id, profile_id, state, active, requested_at, last_alive_at, updated_at)
         VALUES ('x', 'p', 'working', 1, 0, 0, 0)",
        [],
    );
    assert!(constraint_violation(&inserted.unwrap_err()));
    // After a failed start (T3) a new session can begin.
    let aborted = applied(
        store
            .work_session_start_failed(
                first.id,
                &notice(WorkSessionProblem::StartFailed, 3_000),
                3_000,
            )
            .unwrap(),
    );
    assert_eq!(aborted.status, SessionStatus::Aborted);
    assert_eq!(
        aborted.problem.unwrap().problem,
        WorkSessionProblem::StartFailed
    );
    // No interval was ever opened.
    assert_eq!(store.work_session_intervals(first.id).unwrap(), Vec::new());
    assert!(matches!(
        store
            .work_session_start_failed(
                first.id,
                &notice(WorkSessionProblem::StartFailed, 3_000),
                3_000
            )
            .unwrap(),
        Transition::Repeated(_)
    ));
    // With no session active, `active` still follows the state: a finished
    // row cannot be active and an unfinished one must be.
    for (state, active) in [("stopped", "1"), ("working", "NULL")] {
        let inserted = store.connection_for_tests().execute(
            &format!(
                "INSERT INTO work_sessions (id, profile_id, state, active, requested_at, last_alive_at, updated_at)
                 VALUES ('y', 'p', '{state}', {active}, 0, 0, 0)"
            ),
            [],
        );
        assert!(constraint_violation(&inserted.unwrap_err()), "{state}");
    }
    applied(store.begin_work_session(&second).unwrap());
}

#[test]
fn verification_never_stops_the_timer() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let v = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::Working, v, None, 10 * MIN)
        .unwrap();
    let rejected = applied(
        store
            .record_work_session_verification(
                id,
                v,
                &outcome(false, Some("2 tests fail\n")),
                BOOT,
                11 * MIN,
            )
            .unwrap(),
    );
    assert_eq!(rejected.status, SessionStatus::Working);
    assert_eq!(rejected.can_finish, Some(false));
    assert_eq!(
        rejected.verification_details.as_deref(),
        Some("2 tests fail")
    );
    assert_eq!(rejected.verified_at_ms, Some(11 * MIN));
    let problem = rejected.problem.unwrap();
    assert_eq!(problem.problem, WorkSessionProblem::NotReady);
    assert_eq!(problem.message, "2 tests fail");

    let v = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::Working, v, None, 12 * MIN)
        .unwrap();
    let failed = applied(
        store
            .record_work_session_verification(
                id,
                v,
                &VerificationResult::Failed(notice(
                    WorkSessionProblem::VerificationInterrupted,
                    13 * MIN,
                )),
                BOOT,
                13 * MIN,
            )
            .unwrap(),
    );
    assert_eq!(failed.status, SessionStatus::Working);
    assert_eq!(failed.can_finish, None);

    let v = approve(&store, id, 14 * MIN);
    applied(
        store
            .invalidate_work_session_approval(
                id,
                v,
                Some(&notice(WorkSessionProblem::ApprovalExpired, 30 * MIN)),
                30 * MIN,
            )
            .unwrap(),
    );
    // Through T6, T7 and T9 the one interval stays open and time grows.
    let intervals = store.work_session_intervals(id).unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, None);
    assert_eq!(
        store.work_session_elapsed_ms(id, 40 * MIN).unwrap(),
        40 * 60_000
    );
}

#[test]
fn stale_results_and_approvals_change_nothing() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let old = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::Working, old, None, MIN)
        .unwrap();
    store
        .record_work_session_verification(id, old, &outcome(false, None), BOOT, 2 * MIN)
        .unwrap();
    let current = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::Working, current, None, 3 * MIN)
        .unwrap();
    let before = everything(&store, id);
    // A late answer of the older verification.
    assert_eq!(
        store
            .record_work_session_verification(id, old, &outcome(true, None), BOOT, 4 * MIN)
            .unwrap(),
        Transition::Refused(Refusal::StaleVerification)
    );
    // A request from a state the session is no longer in.
    assert_eq!(
        store
            .begin_work_session_verification(
                id,
                SessionStatus::Working,
                VerificationId::new(),
                None,
                4 * MIN
            )
            .unwrap(),
        Transition::Refused(Refusal::WrongState {
            status: SessionStatus::VerifyingEnd
        })
    );
    // Finishing is possible only from readyToFinish.
    assert_eq!(
        store
            .finish_work_session(id, current, BOOT, 4 * MIN)
            .unwrap(),
        Transition::Refused(Refusal::WrongState {
            status: SessionStatus::VerifyingEnd
        })
    );
    assert_eq!(everything(&store, id), before);

    // The same result twice: applied once, then repeated.
    applied(
        store
            .record_work_session_verification(id, current, &outcome(true, None), BOOT, 5 * MIN)
            .unwrap(),
    );
    let after = everything(&store, id);
    assert!(matches!(
        store
            .record_work_session_verification(id, current, &outcome(true, None), BOOT, 6 * MIN)
            .unwrap(),
        Transition::Repeated(_)
    ));
    assert_eq!(everything(&store, id), after);

    // T8: a new verification drops the approval; finishing with it fails.
    let newer = VerificationId::new();
    applied(
        store
            .begin_work_session_verification(id, SessionStatus::ReadyToFinish, newer, None, 7 * MIN)
            .unwrap(),
    );
    let dropped = store.work_session(id).unwrap().unwrap();
    assert_eq!(dropped.approval, None);
    assert_eq!(dropped.can_finish, None);
    store
        .record_work_session_verification(id, newer, &outcome(true, None), BOOT, 8 * MIN)
        .unwrap();
    assert_eq!(
        store
            .finish_work_session(id, current, BOOT, 9 * MIN)
            .unwrap(),
        Transition::Refused(Refusal::StaleVerification)
    );
    // A used verification id cannot be started again.
    assert_eq!(
        store
            .begin_work_session_verification(id, SessionStatus::ReadyToFinish, newer, None, 9 * MIN)
            .unwrap(),
        Transition::Refused(Refusal::StaleVerification)
    );
}

#[test]
fn an_old_sessions_verification_cannot_stop_a_new_one() {
    let (store, profile) = store_with_profile();
    let old = working(&store, &profile, 0);
    let old_approval = approve(&store, old, MIN);
    applied(
        store
            .finish_work_session(old, old_approval, BOOT, 2 * MIN)
            .unwrap(),
    );
    applied(store.complete_work_session_stop(old, 2 * MIN).unwrap());

    let new = working(&store, &profile, 3 * MIN);
    let running = VerificationId::new();
    store
        .begin_work_session_verification(new, SessionStatus::Working, running, None, 4 * MIN)
        .unwrap();
    let before = everything(&store, new);
    assert_eq!(
        store
            .record_work_session_verification(
                new,
                old_approval,
                &outcome(true, None),
                BOOT,
                5 * MIN
            )
            .unwrap(),
        Transition::Refused(Refusal::StaleVerification)
    );
    store
        .record_work_session_verification(new, running, &outcome(true, None), BOOT, 5 * MIN)
        .unwrap();
    let ready = everything(&store, new);
    assert_eq!(
        store
            .finish_work_session(new, old_approval, BOOT, 6 * MIN)
            .unwrap(),
        Transition::Refused(Refusal::StaleVerification)
    );
    assert_eq!(
        store
            .invalidate_work_session_approval(new, old_approval, None, 6 * MIN)
            .unwrap(),
        Transition::Refused(Refusal::StaleVerification)
    );
    assert_eq!(everything(&store, new), ready);
    assert_ne!(before, ready);
    // Requests naming the old session answer "already finished" or refuse;
    // they never touch the new one.
    assert!(matches!(
        store
            .finish_work_session(old, old_approval, BOOT, 6 * MIN)
            .unwrap(),
        Transition::Repeated(_)
    ));
    assert_eq!(everything(&store, new), ready);
}

#[test]
fn the_final_stop_is_atomic_and_idempotent() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let v = approve(&store, id, 10 * MIN);
    let stopping = applied(store.finish_work_session(id, v, BOOT, 11 * MIN).unwrap());
    // A second click, or a duplicated signal, with the same approval.
    for at in [11 * MIN + 1, 12 * MIN] {
        assert_eq!(
            store.finish_work_session(id, v, BOOT, at).unwrap(),
            Transition::Repeated(stopping.clone())
        );
    }
    let stopped = applied(store.complete_work_session_stop(id, 12 * MIN).unwrap());
    assert!(matches!(
        store.complete_work_session_stop(id, 13 * MIN).unwrap(),
        Transition::Repeated(_)
    ));
    assert_eq!(
        store.finish_work_session(id, v, BOOT, 13 * MIN).unwrap(),
        Transition::Repeated(stopped.clone())
    );
    assert_eq!(stopped.stopped_at_ms, Some(11 * MIN));
    assert_eq!(
        store.work_session_elapsed_ms(id, 99 * MIN).unwrap(),
        11 * 60_000
    );
    assert_eq!(store.work_session_intervals(id).unwrap().len(), 1);
    let finishes = store
        .work_session_transitions(id)
        .unwrap()
        .iter()
        .filter(|t| t.event == "finish")
        .count();
    assert_eq!(finishes, 1);
}

#[test]
fn an_approval_holds_for_its_lifetime_and_its_boot_only() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let v = approve(&store, id, 10 * MIN);
    let ready = everything(&store, id);
    // D9: ten minutes by default.
    assert_eq!(
        store
            .work_session_settings(profile.id)
            .unwrap()
            .approval_ttl_seconds,
        600
    );
    assert_eq!(
        store
            .finish_work_session(id, v, BOOT, 20 * MIN + 1)
            .unwrap(),
        Transition::Refused(Refusal::ApprovalExpired)
    );
    // From the future: the clock was set back after the approval.
    assert_eq!(
        store
            .finish_work_session(id, v, BOOT, 10 * MIN - 1)
            .unwrap(),
        Transition::Refused(Refusal::ApprovalExpired)
    );
    assert_eq!(
        store
            .finish_work_session(id, v, "boot-2", 11 * MIN)
            .unwrap(),
        Transition::Refused(Refusal::ApprovalFromAnotherBoot)
    );
    assert_eq!(everything(&store, id), ready);

    // The lifetime is the profile's setting.
    let mut settings = store.work_session_settings(profile.id).unwrap();
    settings.approval_ttl_seconds = 60;
    store
        .save_work_session_settings(&settings, 10 * MIN)
        .unwrap();
    assert_eq!(
        store
            .finish_work_session(id, v, BOOT, 11 * MIN + 1)
            .unwrap(),
        Transition::Refused(Refusal::ApprovalExpired)
    );
    applied(store.finish_work_session(id, v, BOOT, 11 * MIN).unwrap());
}

#[test]
fn invalidating_an_approval_keeps_the_timer_and_is_idempotent() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let v = approve(&store, id, MIN);
    let n = notice(WorkSessionProblem::ApprovalInvalidated, 2 * MIN);
    let record = applied(
        store
            .invalidate_work_session_approval(id, v, Some(&n), 2 * MIN)
            .unwrap(),
    );
    assert_eq!(record.status, SessionStatus::Working);
    assert_eq!(record.approval, None);
    assert_eq!(record.problem, Some(n.clone()));
    assert!(matches!(
        store
            .invalidate_work_session_approval(id, v, Some(&n), 3 * MIN)
            .unwrap(),
        Transition::Repeated(_)
    ));
    assert_eq!(
        store.finish_work_session(id, v, BOOT, 3 * MIN).unwrap(),
        Transition::Refused(Refusal::WrongState {
            status: SessionStatus::Working
        })
    );
    assert!(store.clear_work_session_problem(id).unwrap());
    assert!(!store.clear_work_session_problem(id).unwrap());
}

#[test]
fn notices_must_fit_their_transition() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let v = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::Working, v, None, MIN)
        .unwrap();
    let before = everything(&store, id);
    let wrong = store.record_work_session_verification(
        id,
        v,
        &VerificationResult::Failed(notice(WorkSessionProblem::StartFailed, MIN)),
        BOOT,
        MIN,
    );
    assert!(matches!(wrong, Err(StorageError::Invalid(_))), "{wrong:?}");
    assert!(matches!(
        store.work_session_start_failed(id, &notice(WorkSessionProblem::NotReady, MIN), MIN),
        Err(StorageError::Invalid(_))
    ));
    assert_eq!(everything(&store, id), before);
    assert_eq!(
        store
            .work_session_start_failed(id, &notice(WorkSessionProblem::StartFailed, MIN), MIN)
            .unwrap(),
        Transition::Refused(Refusal::WrongState {
            status: SessionStatus::VerifyingEnd
        })
    );
    assert_eq!(
        store
            .work_session_start_succeeded(WorkSessionId::new(), MIN)
            .unwrap(),
        Transition::Refused(Refusal::UnknownSession)
    );
}

#[test]
fn the_heartbeat_writes_no_counter() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let before = store.work_session(id).unwrap().unwrap();
    let intervals = store.work_session_intervals(id).unwrap();
    for minute in 1..=5 {
        assert!(store.touch_work_session(id, minute * MIN).unwrap());
    }
    let after = store.work_session(id).unwrap().unwrap();
    assert_eq!(after.last_alive_at_ms, 5 * MIN);
    assert_eq!(
        WorkSessionRecord {
            last_alive_at_ms: before.last_alive_at_ms,
            ..after
        },
        before
    );
    assert_eq!(store.work_session_intervals(id).unwrap(), intervals);
    assert_eq!(
        store.work_session_elapsed_ms(id, 5 * MIN).unwrap(),
        5 * 60_000
    );
    // Only the active session has a heartbeat.
    assert!(
        !store
            .touch_work_session(WorkSessionId::new(), 6 * MIN)
            .unwrap()
    );
}

#[test]
fn long_gaps_are_counted_flagged_and_can_be_excluded() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    // At most 15 minutes (D8): nothing to review.
    assert_eq!(
        store
            .flag_work_session_gap(id, GapKind::Sleep, 10 * MIN, 25 * MIN, 25 * MIN)
            .unwrap(),
        None
    );
    assert_eq!(store.work_session(id).unwrap().unwrap().problem, None);
    let review = store
        .flag_work_session_gap(id, GapKind::AgentDown, 30 * MIN, 50 * MIN, 50 * MIN)
        .unwrap()
        .unwrap();
    assert_eq!(review.kind, GapKind::AgentDown);
    assert_eq!(review.resolution, None);
    let session = store.work_session(id).unwrap().unwrap();
    assert_eq!(
        session.problem.unwrap().problem,
        WorkSessionProblem::NeedsReview
    );
    // Counted by default.
    assert_eq!(
        store.work_session_elapsed_ms(id, 60 * MIN).unwrap(),
        60 * 60_000
    );
    let history = store
        .work_session_history(
            HistoryQuery {
                profile: None,
                before_ms: None,
                limit: 5,
            },
            60 * MIN,
        )
        .unwrap();
    assert_eq!(history[0].open_reviews, 1);

    // Excluding splits the open interval; the timer runs on.
    let excluded = store
        .resolve_work_session_review(review.id, ReviewResolution::Excluded, 61 * MIN)
        .unwrap();
    assert_eq!(excluded.resolution, Some(ReviewResolution::Excluded));
    assert_eq!(
        store.work_session_elapsed_ms(id, 60 * MIN).unwrap(),
        40 * 60_000
    );
    let intervals = store.work_session_intervals(id).unwrap();
    assert_eq!(intervals.len(), 2);
    assert_eq!(intervals[0].interval.ended_at_ms, Some(30 * MIN));
    assert_eq!(intervals[0].end_reason, Some(IntervalEnd::GapExcluded));
    assert_eq!(intervals[1].interval.started_at_ms, 50 * MIN);
    assert_eq!(intervals[1].interval.ended_at_ms, None);
    // The notice goes once no review is open.
    assert_eq!(store.work_session(id).unwrap().unwrap().problem, None);
    // Again the same way: nothing changes.
    let before = everything(&store, id);
    store
        .resolve_work_session_review(review.id, ReviewResolution::Excluded, 62 * MIN)
        .unwrap();
    assert_eq!(everything(&store, id), before);

    // Counting it again joins the intervals back into one open interval.
    let included = store
        .resolve_work_session_review(review.id, ReviewResolution::Counted, 63 * MIN)
        .unwrap();
    assert_eq!(included.resolution, Some(ReviewResolution::Counted));
    let intervals = store.work_session_intervals(id).unwrap();
    assert_eq!(intervals.len(), 1, "{intervals:?}");
    assert_eq!(intervals[0].interval.started_at_ms, 0);
    assert_eq!(intervals[0].interval.ended_at_ms, None);
    assert_eq!(
        store.work_session_elapsed_ms(id, 60 * MIN).unwrap(),
        60 * 60_000
    );
    // And it can be excluded again.
    store
        .resolve_work_session_review(review.id, ReviewResolution::Excluded, 64 * MIN)
        .unwrap();
    assert_eq!(
        store.work_session_elapsed_ms(id, 60 * MIN).unwrap(),
        40 * 60_000
    );
}

#[test]
fn re_including_a_gap_keeps_other_exclusions_and_the_stop() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let first = store
        .flag_work_session_gap(id, GapKind::Sleep, 10 * MIN, 30 * MIN, 30 * MIN)
        .unwrap()
        .unwrap();
    let overlapping = store
        .flag_work_session_gap(id, GapKind::AgentDown, 20 * MIN, 40 * MIN, 40 * MIN)
        .unwrap()
        .unwrap();
    let v = approve(&store, id, 60 * MIN);
    applied(store.finish_work_session(id, v, BOOT, 60 * MIN).unwrap());
    applied(store.complete_work_session_stop(id, 60 * MIN).unwrap());
    for review in [&first, &overlapping] {
        store
            .resolve_work_session_review(review.id, ReviewResolution::Excluded, 70 * MIN)
            .unwrap();
    }
    assert_eq!(
        store.work_session_elapsed_ms(id, 99 * MIN).unwrap(),
        30 * 60_000
    );
    // Re-including the first counts only what the second does not exclude.
    store
        .resolve_work_session_review(first.id, ReviewResolution::Counted, 71 * MIN)
        .unwrap();
    assert_eq!(
        store.work_session_elapsed_ms(id, 99 * MIN).unwrap(),
        40 * 60_000
    );
    store
        .resolve_work_session_review(overlapping.id, ReviewResolution::Counted, 72 * MIN)
        .unwrap();
    assert_eq!(
        store.work_session_elapsed_ms(id, 99 * MIN).unwrap(),
        60 * 60_000
    );
    let intervals = store.work_session_intervals(id).unwrap();
    assert_eq!(intervals.len(), 1);
    assert_eq!(intervals[0].interval.ended_at_ms, Some(60 * MIN));
    assert_eq!(intervals[0].end_reason, Some(IntervalEnd::Stopped));
    // The stop time is never moved by re-including.
    assert_eq!(
        store.work_session(id).unwrap().unwrap().stopped_at_ms,
        Some(60 * MIN)
    );
}

#[test]
fn an_idle_snapshot_shows_and_clears_the_last_notice() {
    let (store, profile) = store_with_profile();
    assert_eq!(store.idle_work_session_notice().unwrap(), None);
    let new = new_session(&profile, 0);
    applied(store.begin_work_session(&new).unwrap());
    // While a session is active, its own row holds its notice.
    assert_eq!(store.idle_work_session_notice().unwrap(), None);
    let failed = notice(WorkSessionProblem::StartFailed, MIN);
    applied(
        store
            .work_session_start_failed(new.id, &failed, MIN)
            .unwrap(),
    );
    assert_eq!(
        store.idle_work_session_notice().unwrap(),
        Some((new.id, failed))
    );
    assert_eq!(store.clear_idle_work_session_notices().unwrap(), 1);
    assert_eq!(store.idle_work_session_notice().unwrap(), None);
    assert_eq!(store.clear_idle_work_session_notices().unwrap(), 0);
}

#[test]
fn gaps_can_be_excluded_after_the_stop_and_across_intervals() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let first = store
        .flag_work_session_gap(id, GapKind::Sleep, 10 * MIN, 30 * MIN, 30 * MIN)
        .unwrap()
        .unwrap();
    let wide = store
        .flag_work_session_gap(id, GapKind::ClockForward, 5 * MIN, 40 * MIN, 40 * MIN)
        .unwrap()
        .unwrap();
    let v = approve(&store, id, 60 * MIN);
    applied(store.finish_work_session(id, v, BOOT, 60 * MIN).unwrap());
    applied(store.complete_work_session_stop(id, 60 * MIN).unwrap());

    store
        .resolve_work_session_review(first.id, ReviewResolution::Excluded, 70 * MIN)
        .unwrap();
    assert_eq!(
        store.work_session_elapsed_ms(id, 99 * MIN).unwrap(),
        40 * 60_000
    );
    // Overlapping the first exclusion: 5..10 and 30..40 go too.
    store
        .resolve_work_session_review(wide.id, ReviewResolution::Excluded, 71 * MIN)
        .unwrap();
    assert_eq!(
        store.work_session_elapsed_ms(id, 99 * MIN).unwrap(),
        25 * 60_000
    );
    let intervals = store.work_session_intervals(id).unwrap();
    assert!(intervals.iter().all(|i| i.interval.ended_at_ms.is_some()));
    assert_eq!(
        intervals.last().unwrap().end_reason,
        Some(IntervalEnd::Stopped)
    );
}

#[test]
fn a_clock_set_back_is_flagged_but_never_excluded() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 100 * MIN);
    let review = store
        .flag_work_session_gap(id, GapKind::ClockBack, 120 * MIN, 90 * MIN, 90 * MIN)
        .unwrap()
        .unwrap();
    assert!(matches!(
        store.resolve_work_session_review(review.id, ReviewResolution::Excluded, 91 * MIN),
        Err(StorageError::Invalid(_))
    ));
    store
        .resolve_work_session_review(review.id, ReviewResolution::Counted, 91 * MIN)
        .unwrap();
    // A gap that ends before it begins must be a clock set back.
    assert!(matches!(
        store.flag_work_session_gap(id, GapKind::Sleep, 120 * MIN, 90 * MIN, 90 * MIN),
        Err(StorageError::Invalid(_))
    ));
}

#[test]
fn no_gap_is_flagged_while_the_timer_does_not_run() {
    let (store, profile) = store_with_profile();
    let new = new_session(&profile, 0);
    applied(store.begin_work_session(&new).unwrap());
    assert_eq!(
        store
            .flag_work_session_gap(new.id, GapKind::AgentDown, 0, 60 * MIN, 60 * MIN)
            .unwrap(),
        None
    );
    assert_eq!(store.work_session_reviews(new.id).unwrap(), Vec::new());
}

#[test]
fn settings_have_defaults_bounds_and_belong_to_their_profile() {
    let (store, profile) = store_with_profile();
    let defaults = store.work_session_settings(profile.id).unwrap();
    assert_eq!(defaults, WorkSessionSettings::defaults(profile.id));
    assert_eq!(defaults.review_gap_seconds, 900);
    assert_eq!(defaults.elapsed_format, ElapsedFormat::Minutes);

    let settings = WorkSessionSettings {
        start_workflow: Some(WorkflowId::new()),
        verification_workflow: Some(WorkflowId::new()),
        elapsed_format: ElapsedFormat::Seconds,
        ..defaults.clone()
    };
    let saved = store.save_work_session_settings(&settings, 1_234).unwrap();
    assert_eq!(saved.updated_at_ms, Some(1_234));
    assert_eq!(
        store.work_session_settings(profile.id).unwrap(),
        WorkSessionSettings {
            updated_at_ms: Some(1_234),
            ..settings.clone()
        }
    );
    for bad in [59, 86_401] {
        let refused = store.save_work_session_settings(
            &WorkSessionSettings {
                approval_ttl_seconds: bad,
                ..settings.clone()
            },
            1,
        );
        assert!(matches!(refused, Err(StorageError::Invalid(_))), "{bad}");
    }
    assert!(matches!(
        store.save_work_session_settings(&WorkSessionSettings::defaults(ProfileId::new()), 1),
        Err(StorageError::UnknownProfile)
    ));
    // The settings go with the profile.
    store.delete_profile(profile.id).unwrap();
    assert_eq!(
        store.work_session_settings(profile.id).unwrap(),
        WorkSessionSettings::defaults(profile.id)
    );
}

#[test]
fn a_profile_with_the_active_session_cannot_be_deleted() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    assert!(matches!(
        store.delete_profile(profile.id),
        Err(StorageError::WorkSessionActive)
    ));
    assert!(matches!(
        store.apply_changes(&ConfigChanges {
            delete_profiles: vec![profile.id],
            ..ConfigChanges::default()
        }),
        Err(StorageError::WorkSessionActive)
    ));
    assert!(store.load_profile(profile.id).unwrap().is_some());

    let v = approve(&store, id, MIN);
    applied(store.finish_work_session(id, v, BOOT, MIN).unwrap());
    applied(store.complete_work_session_stop(id, MIN).unwrap());
    assert!(store.delete_profile(profile.id).unwrap());
    // The history outlives the profile.
    assert_eq!(
        store.work_session(id).unwrap().unwrap().status,
        SessionStatus::Stopped
    );
    assert_eq!(store.work_session_intervals(id).unwrap().len(), 1);
}

#[test]
fn history_is_newest_first_per_profile_and_limited() {
    let (store, profile) = store_with_profile();
    let other = Profile::new("Other");
    store.save_profile(&other).unwrap();
    let mut ids = Vec::new();
    for (index, owner) in [&profile, &other, &profile].into_iter().enumerate() {
        let at = i64::try_from(index).unwrap() * 100 * MIN;
        let id = working(&store, owner, at + 1000);
        let v = approve(&store, id, at + 10 * MIN);
        applied(
            store
                .finish_work_session(id, v, BOOT, at + 10 * MIN)
                .unwrap(),
        );
        applied(store.complete_work_session_stop(id, at + 10 * MIN).unwrap());
        ids.push(id);
    }
    let query = |profile: Option<ProfileId>, before_ms: Option<i64>| {
        store
            .work_session_history(
                HistoryQuery {
                    profile,
                    before_ms,
                    limit: 10,
                },
                0,
            )
            .unwrap()
            .into_iter()
            .map(|summary| summary.session.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(query(None, None), [ids[2], ids[1], ids[0]]);
    assert_eq!(query(Some(profile.id), None), [ids[2], ids[0]]);
    assert_eq!(query(None, Some(200 * MIN)), [ids[1], ids[0]]);
}

#[test]
fn only_the_newest_finished_sessions_are_kept() {
    let (store, profile) = store_with_profile();
    let mut first = None;
    for index in 0..=KEPT_SESSIONS {
        let new = new_session(&profile, i64::try_from(index).unwrap());
        applied(store.begin_work_session(&new).unwrap());
        applied(
            store
                .work_session_start_failed(new.id, &notice(WorkSessionProblem::StartFailed, 0), 0)
                .unwrap(),
        );
        first.get_or_insert(new.id);
    }
    let count: i64 = store
        .connection_for_tests()
        .query_row("SELECT COUNT(*) FROM work_sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, i64::try_from(KEPT_SESSIONS).unwrap());
    let first = first.unwrap();
    assert_eq!(store.work_session(first).unwrap(), None);
    // With its log.
    assert_eq!(store.work_session_transitions(first).unwrap(), Vec::new());
    // The active session is never pruned.
    let active = working(&store, &profile, 10_000_000);
    let new = new_session(&profile, 0);
    assert!(matches!(
        store.begin_work_session(&new).unwrap(),
        Transition::Refused(_)
    ));
    assert!(store.work_session(active).unwrap().is_some());
}

fn temp_database() -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("deckpilot-work-session-{}", ProfileId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("deckpilot.db");
    (dir, path)
}

/// Recovery after a restart in each persisted state (§6.5): the store
/// keeps everything the next agent needs, and the recovery transitions
/// never lose counted time.
#[test]
#[allow(clippy::too_many_lines)] // one restart and one recovery per persisted state
fn every_persisted_state_survives_a_restart_and_recovers_without_losing_time() {
    use deckpilot_core::work_session::WorkSessionState as S;
    let (dir, path) = temp_database();
    let open = || SqliteStore::open(&path, &Encryption::None).unwrap();
    let mut profile = Profile::new("Work");
    profile
        .projects
        .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
    open().save_profile(&profile).unwrap();

    for state in [
        S::Starting,
        S::Working,
        S::VerifyingEnd,
        S::ReadyToFinish,
        S::Stopping,
    ] {
        let store = open();
        let new = new_session(&profile, 0);
        applied(store.begin_work_session(&new).unwrap());
        let id = new.id;
        let mut approval = None;
        let verification = VerificationId::new();
        if state != S::Starting {
            applied(store.work_session_start_succeeded(id, MIN).unwrap());
            store.touch_work_session(id, 2 * MIN).unwrap();
        }
        if matches!(state, S::VerifyingEnd | S::ReadyToFinish | S::Stopping) {
            store
                .begin_work_session_verification(
                    id,
                    SessionStatus::Working,
                    verification,
                    None,
                    2 * MIN,
                )
                .unwrap();
        }
        if matches!(state, S::ReadyToFinish | S::Stopping) {
            store
                .record_work_session_verification(
                    id,
                    verification,
                    &outcome(true, None),
                    BOOT,
                    3 * MIN,
                )
                .unwrap();
            approval = Some(verification);
        }
        if state == S::Stopping {
            applied(
                store
                    .finish_work_session(id, verification, BOOT, 4 * MIN)
                    .unwrap(),
            );
        }
        let before = everything(&store, id);
        // The process dies here.
        drop(store);

        let store = open();
        assert_eq!(everything(&store, id), before, "{state:?}");
        let active = store.active_work_session().unwrap().unwrap();
        assert_eq!(active.id, id);
        assert_eq!(active.status.state(), state);
        let now = 60 * MIN;
        let recovered = match state {
            S::Starting => applied(
                store
                    .work_session_start_failed(
                        id,
                        &notice(WorkSessionProblem::StartInterrupted, now),
                        now,
                    )
                    .unwrap(),
            ),
            S::Working => {
                // The agent was down from the last heartbeat until now.
                let review = store
                    .flag_work_session_gap(
                        id,
                        GapKind::AgentDown,
                        active.last_alive_at_ms,
                        now,
                        now,
                    )
                    .unwrap();
                assert!(review.is_some());
                store.work_session(id).unwrap().unwrap()
            }
            S::VerifyingEnd => applied(
                store
                    .record_work_session_verification(
                        id,
                        verification,
                        &VerificationResult::Failed(notice(
                            WorkSessionProblem::VerificationInterrupted,
                            now,
                        )),
                        "boot-2",
                        now,
                    )
                    .unwrap(),
            ),
            S::ReadyToFinish => {
                // The new instance cannot finish with the old approval.
                assert_eq!(
                    store
                        .finish_work_session(id, verification, "boot-2", 3 * MIN + 1)
                        .unwrap(),
                    Transition::Refused(Refusal::ApprovalFromAnotherBoot)
                );
                applied(
                    store
                        .invalidate_work_session_approval(
                            id,
                            approval.unwrap(),
                            Some(&notice(WorkSessionProblem::ApprovalInvalidated, now)),
                            now,
                        )
                        .unwrap(),
                )
            }
            S::Stopping => applied(store.complete_work_session_stop(id, now).unwrap()),
            S::Idle => unreachable!(),
        };
        let elapsed = store.work_session_elapsed_ms(id, now).unwrap();
        match state {
            S::Starting => {
                assert_eq!(recovered.status, SessionStatus::Aborted);
                assert_eq!(elapsed, 0);
            }
            S::Stopping => {
                assert_eq!(recovered.status, SessionStatus::Stopped);
                assert_eq!(recovered.stopped_at_ms, Some(4 * MIN));
                assert_eq!(elapsed, 3 * 60_000);
            }
            _ => {
                // The timer ran on through the restart.
                assert_eq!(recovered.status, SessionStatus::Working, "{state:?}");
                assert_eq!(elapsed, 59 * 60_000, "{state:?}");
                // Finish it so the next state can start a session.
                let v = approve(&store, id, now);
                applied(store.finish_work_session(id, v, BOOT, now).unwrap());
                applied(store.complete_work_session_stop(id, now).unwrap());
            }
        }
        assert_eq!(store.active_work_session().unwrap(), None, "{state:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn two_connections_cannot_both_finish() {
    let (dir, path) = temp_database();
    let first = SqliteStore::open(&path, &Encryption::None).unwrap();
    let profile = Profile::new("Work");
    first.save_profile(&profile).unwrap();
    let id = working(&first, &profile, 0);
    let v = approve(&first, id, MIN);
    // The agent and the app are separate processes with their own
    // connections; here two connections on two threads.
    let finish = |at: i64| {
        let path = path.clone();
        std::thread::spawn(move || {
            SqliteStore::open(&path, &Encryption::None)
                .unwrap()
                .finish_work_session(id, v, BOOT, at)
                .unwrap()
        })
    };
    let (a, b) = (finish(2 * MIN), finish(2 * MIN + 7));
    let results = [a.join().unwrap(), b.join().unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Transition::Applied(_)))
            .count(),
        1,
        "{results:?}"
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Transition::Repeated(_)))
            .count(),
        1
    );
    let stopped_at = first.work_session(id).unwrap().unwrap().stopped_at_ms;
    assert!(matches!(stopped_at, Some(at) if at == 2 * MIN || at == 2 * MIN + 7));
    drop(first);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn details_are_cleaned_and_bounded() {
    let (store, profile) = store_with_profile();
    let id = working(&store, &profile, 0);
    let v = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::Working, v, None, MIN)
        .unwrap();
    let long = format!("  {}\u{7}", "x".repeat(500));
    let record = applied(
        store
            .record_work_session_verification(id, v, &outcome(false, Some(&long)), BOOT, MIN)
            .unwrap(),
    );
    assert_eq!(
        record.verification_details.as_deref().map(str::len),
        Some(198)
    );
    // `set_verification_execution` only for the running verification.
    assert!(
        !store
            .set_work_session_verification_execution(id, v, ExecutionId::new())
            .unwrap()
    );
}
