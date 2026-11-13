//! Faces of work session keys (`docs/gestures-and-work-session.md`,
//! section 8.2): a key bound to `Stateful { machine: "workSession" }`
//! shows the session's state, read synchronously from the agent's
//! in-memory [`WorkSessionSnapshot`] while the key is drawn. There is no
//! provider thread and no polling.
//!
//! - **Faces.** Label, detail and indicator per state, as in the table of
//!   section 8.2. The binding's appearance (colours, title size and
//!   placement, icon) is kept; a hint the key's last gesture left
//!   ("Hold to finish", …) replaces the detail line.
//! - **Elapsed time.** Transitions redraw by themselves
//!   ([`Agent::work_session_changed`]). The elapsed text is the only thing
//!   that ticks: `h:mm` by default, redrawn at each minute boundary of the
//!   elapsed time, or `h:mm:ss` every second when the profile opts in
//!   (decision D4). The redraw is one deadline in the run loop, kept only
//!   while at least one work session key is shown on a connected device
//!   and the timer runs. It is a wall-clock time, so a computer that slept
//!   or a clock that jumped redraws at once. Only devices that show such a
//!   key are redrawn, and only keys whose face changed are sent.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use deckpilot_core::work_session::{WorkSessionProblem, WorkSessionSnapshot, WorkSessionState};
use deckpilot_core::{Indicator, KeyFace};
use deckpilot_device::{ControlId, DeviceId};
use deckpilot_storage::work_sessions::ElapsedFormat;

use super::Agent;

const MINUTE_MS: u64 = 60_000;
const SECOND_MS: u64 = 1000;

/// The work session keys shown on each connected device, and when their
/// elapsed text changes next.
#[derive(Debug, Default)]
pub(super) struct WorkSessionFaces {
    shown: BTreeMap<DeviceId, BTreeSet<ControlId>>,
    /// Wall-clock milliseconds at which the elapsed text drawn last
    /// changes; `None` while the timer stands.
    due_ms: Option<i64>,
}

impl WorkSessionFaces {
    /// Replaces the work session keys shown on `device`.
    pub(super) fn set_shown(&mut self, device: &DeviceId, keys: BTreeSet<ControlId>) {
        if keys.is_empty() {
            self.shown.remove(device);
        } else {
            self.shown.insert(device.clone(), keys);
        }
    }

    pub(super) fn forget_device(&mut self, device: &DeviceId) {
        self.shown.remove(device);
    }

    /// When the elapsed text changes next, if a work session key is shown.
    pub(super) fn due_ms(&self) -> Option<i64> {
        if self.shown.is_empty() {
            None
        } else {
            self.due_ms
        }
    }
}

/// How often the elapsed text changes.
fn period_ms(format: ElapsedFormat) -> u64 {
    match format {
        ElapsedFormat::Minutes => MINUTE_MS,
        ElapsedFormat::Seconds => SECOND_MS,
    }
}

/// Elapsed time as `h:mm`, or `h:mm:ss`.
pub(crate) fn elapsed_text(elapsed_ms: u64, format: ElapsedFormat) -> String {
    let seconds = elapsed_ms / SECOND_MS;
    let (hours, minutes) = (seconds / 3600, seconds / 60 % 60);
    match format {
        ElapsedFormat::Minutes => format!("{hours}:{minutes:02}"),
        ElapsedFormat::Seconds => format!("{hours}:{minutes:02}:{:02}", seconds % 60),
    }
}

/// Counted time of `snapshot` at `now_ms`: the published value, moved on
/// while the timer runs.
pub(crate) fn elapsed_at(snapshot: &WorkSessionSnapshot, now_ms: i64) -> u64 {
    if !snapshot.state.timer_running() {
        return snapshot.elapsed_ms;
    }
    let since = u64::try_from(now_ms.saturating_sub(snapshot.as_of_ms)).unwrap_or(0);
    snapshot.elapsed_ms.saturating_add(since)
}

/// When the elapsed text of `snapshot` changes after `now_ms` (wall-clock
/// milliseconds); `None` while the timer stands.
pub(crate) fn next_change_ms(
    snapshot: &WorkSessionSnapshot,
    now_ms: i64,
    format: ElapsedFormat,
) -> Option<i64> {
    if !snapshot.state.timer_running() {
        return None;
    }
    let period = period_ms(format);
    let elapsed = elapsed_at(snapshot, now_ms);
    let left = period - elapsed % period;
    Some(now_ms.saturating_add(i64::try_from(left).unwrap_or(i64::MAX)))
}

/// A notice, short enough for a key.
fn notice_text(problem: WorkSessionProblem) -> &'static str {
    match problem {
        WorkSessionProblem::StartFailed => "Start failed",
        WorkSessionProblem::StartInterrupted | WorkSessionProblem::VerificationInterrupted => {
            "Interrupted"
        }
        WorkSessionProblem::VerificationFailed => "Check failed",
        WorkSessionProblem::NotReady => "Not ready",
        WorkSessionProblem::ApprovalExpired => "Expired",
        WorkSessionProblem::ApprovalInvalidated => "Check again",
        WorkSessionProblem::NeedsReview => "Review",
    }
}

/// The face of a work session key: `base` (the binding's face, with its
/// appearance) showing `snapshot` at `now_ms`, with `hint` as the detail
/// line.
pub(crate) fn work_session_face(
    base: KeyFace,
    snapshot: &WorkSessionSnapshot,
    now_ms: i64,
    format: ElapsedFormat,
    hint: Option<&str>,
) -> KeyFace {
    let elapsed = || elapsed_text(elapsed_at(snapshot, now_ms), format);
    let last_notice = snapshot.notices.last().map(|n| notice_text(n.problem));
    let (title, detail, indicator) = match snapshot.state {
        WorkSessionState::Idle => (
            "Start Work".to_owned(),
            last_notice.map(str::to_owned),
            last_notice.map(|_| Indicator::Warning),
        ),
        WorkSessionState::Starting => ("Starting…".to_owned(), None, Some(Indicator::Running)),
        WorkSessionState::Working => (
            elapsed(),
            Some("Working".to_owned()),
            Some(if snapshot.notices.is_empty() {
                Indicator::Ok
            } else {
                Indicator::Warning
            }),
        ),
        WorkSessionState::VerifyingEnd => (
            "Checking…".to_owned(),
            Some(elapsed()),
            Some(Indicator::Running),
        ),
        WorkSessionState::ReadyToFinish => (
            "Finish Work".to_owned(),
            Some(elapsed()),
            Some(Indicator::Ok),
        ),
        WorkSessionState::Stopping => ("Stopping…".to_owned(), None, Some(Indicator::Running)),
    };
    KeyFace {
        title,
        detail: hint.map(str::to_owned).or(detail),
        indicator,
        progress: None,
        toggle: None,
        ..base
    }
}

impl Agent {
    /// The face of the work session key `control` of `device`, which is
    /// shown: `base` with the session's state and the key's hint.
    pub(super) fn work_session_key_face(
        &mut self,
        device: &DeviceId,
        control: ControlId,
        base: KeyFace,
    ) -> KeyFace {
        let snapshot = self.work_session_snapshot();
        let now = self.wall_ms();
        let format = self.work.elapsed_format();
        self.session_faces.due_ms = next_change_ms(&snapshot, now, format);
        let hint = self.stateful_hint(device, control).map(str::to_owned);
        work_session_face(base, &snapshot, now, format, hint.as_deref())
    }

    /// When the run loop should wake up to redraw elapsed time: `None`
    /// unless a work session key is shown and its timer runs.
    pub(super) fn next_work_session_face_due(&self) -> Option<Instant> {
        let due = self.session_faces.due_ms()?;
        let left = u64::try_from(due.saturating_sub(self.wall_ms())).unwrap_or(0);
        Some(Instant::now() + Duration::from_millis(left))
    }

    /// The elapsed text changed: redraws the devices that show a work
    /// session key (only changed faces are sent).
    pub(super) fn on_work_session_face_due(&mut self) {
        let devices: Vec<DeviceId> = self.session_faces.shown.keys().cloned().collect();
        // Rescheduled by drawing; nothing again if no key is drawn.
        self.session_faces.due_ms = None;
        for device in devices {
            self.render(&device, false);
        }
    }

    /// When the elapsed text is redrawn next (wall-clock ms), for tests.
    #[cfg(feature = "test-hooks")]
    pub(super) fn work_session_face_due(&self) -> Option<i64> {
        self.session_faces.due_ms()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::work_session::WorkSessionNotice;
    use deckpilot_core::{Color, FaceStyle, KeyAppearance, WorkSessionId};

    const T0: i64 = 1_800_000_000_000;

    fn snapshot(state: WorkSessionState, elapsed_ms: u64) -> WorkSessionSnapshot {
        let mut snapshot = WorkSessionSnapshot::idle(T0);
        snapshot.state = state;
        snapshot.elapsed_ms = elapsed_ms;
        if state.has_session() {
            snapshot.session = Some(WorkSessionId::new());
        }
        snapshot
    }

    fn base() -> KeyFace {
        KeyFace::new(FaceStyle::Action, "Work Session")
    }

    fn face(state: WorkSessionState, elapsed_ms: u64) -> KeyFace {
        work_session_face(
            base(),
            &snapshot(state, elapsed_ms),
            T0,
            ElapsedFormat::Minutes,
            None,
        )
    }

    fn shown(face: &KeyFace) -> (&str, Option<&str>, Option<Indicator>) {
        (face.title.as_str(), face.detail.as_deref(), face.indicator)
    }

    #[test]
    fn every_state_has_its_face() {
        use Indicator::{Ok, Running};
        use WorkSessionState as S;
        let hour = 3_900_000; // 1:05
        assert_eq!(shown(&face(S::Idle, 0)), ("Start Work", None, None));
        assert_eq!(
            shown(&face(S::Starting, 0)),
            ("Starting…", None, Some(Running))
        );
        assert_eq!(
            shown(&face(S::Working, hour)),
            ("1:05", Some("Working"), Some(Ok))
        );
        assert_eq!(
            shown(&face(S::VerifyingEnd, hour)),
            ("Checking…", Some("1:05"), Some(Running))
        );
        assert_eq!(
            shown(&face(S::ReadyToFinish, hour)),
            ("Finish Work", Some("1:05"), Some(Ok))
        );
        assert_eq!(
            shown(&face(S::Stopping, hour)),
            ("Stopping…", None, Some(Running))
        );
    }

    #[test]
    fn notices_warn() {
        let notice = |problem| WorkSessionNotice {
            problem,
            message: "a long message for the app".into(),
            at_ms: T0,
        };
        let mut idle = snapshot(WorkSessionState::Idle, 0);
        idle.notices.push(notice(WorkSessionProblem::StartFailed));
        let face = work_session_face(base(), &idle, T0, ElapsedFormat::Minutes, None);
        assert_eq!(
            shown(&face),
            ("Start Work", Some("Start failed"), Some(Indicator::Warning))
        );
        let mut working = snapshot(WorkSessionState::Working, 0);
        working
            .notices
            .push(notice(WorkSessionProblem::NeedsReview));
        let face = work_session_face(base(), &working, T0, ElapsedFormat::Minutes, None);
        assert_eq!(
            shown(&face),
            ("0:00", Some("Working"), Some(Indicator::Warning))
        );
    }

    #[test]
    fn hints_replace_the_detail_and_appearance_is_kept() {
        let appearance = KeyAppearance {
            background: Some(Color([1, 2, 3])),
            ..KeyAppearance::default()
        };
        let base = base().with_appearance(Some(&appearance));
        let ready = snapshot(WorkSessionState::ReadyToFinish, 0);
        let face = work_session_face(
            base,
            &ready,
            T0,
            ElapsedFormat::Minutes,
            Some("Hold to finish"),
        );
        assert_eq!(face.detail.as_deref(), Some("Hold to finish"));
        assert_eq!(face.appearance, Some(appearance));
        assert_eq!(face.style, FaceStyle::Action);
    }

    #[test]
    fn elapsed_is_h_mm_or_with_seconds() {
        assert_eq!(elapsed_text(0, ElapsedFormat::Minutes), "0:00");
        assert_eq!(elapsed_text(59_999, ElapsedFormat::Minutes), "0:00");
        assert_eq!(elapsed_text(60_000, ElapsedFormat::Minutes), "0:01");
        assert_eq!(elapsed_text(3_900_000, ElapsedFormat::Minutes), "1:05");
        assert_eq!(elapsed_text(36_000_000, ElapsedFormat::Minutes), "10:00");
        assert_eq!(elapsed_text(3_907_500, ElapsedFormat::Seconds), "1:05:07");
    }

    #[test]
    fn elapsed_moves_on_from_the_snapshot_while_the_timer_runs() {
        let working = snapshot(WorkSessionState::Working, 59_000);
        assert_eq!(elapsed_at(&working, T0 + 2000), 61_000);
        let stopping = snapshot(WorkSessionState::Stopping, 59_000);
        assert_eq!(elapsed_at(&stopping, T0 + 2000), 59_000);
        // A clock set back counts nothing.
        assert_eq!(elapsed_at(&working, T0 - 5000), 59_000);
    }

    #[test]
    fn the_next_change_is_the_next_boundary_of_elapsed_time() {
        let working = snapshot(WorkSessionState::Working, 59_000);
        assert_eq!(
            next_change_ms(&working, T0, ElapsedFormat::Minutes),
            Some(T0 + 1000)
        );
        assert_eq!(
            next_change_ms(&working, T0 + 1000, ElapsedFormat::Minutes),
            Some(T0 + 61_000)
        );
        assert_eq!(
            next_change_ms(&working, T0 + 250, ElapsedFormat::Seconds),
            Some(T0 + 1000)
        );
        for state in [
            WorkSessionState::Idle,
            WorkSessionState::Starting,
            WorkSessionState::Stopping,
        ] {
            assert_eq!(
                next_change_ms(&snapshot(state, 0), T0, ElapsedFormat::Seconds),
                None
            );
        }
    }

    #[test]
    fn nothing_is_due_without_a_shown_key() {
        let device = DeviceId {
            model: deckpilot_device::DeviceModelId::new("test.model"),
            serial: "A".into(),
        };
        let mut faces = WorkSessionFaces {
            due_ms: Some(T0),
            ..WorkSessionFaces::default()
        };
        assert_eq!(faces.due_ms(), None);
        faces.set_shown(&device, BTreeSet::from([ControlId::Key { index: 0 }]));
        assert_eq!(faces.due_ms(), Some(T0));
        faces.set_shown(&device, BTreeSet::new());
        assert_eq!(faces.due_ms(), None);
        faces.set_shown(&device, BTreeSet::from([ControlId::Key { index: 0 }]));
        faces.forget_device(&device);
        assert_eq!(faces.due_ms(), None);
    }
}
