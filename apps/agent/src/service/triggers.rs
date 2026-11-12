//! Workflows started by schedules and lifecycle events.
//!
//! One [`Scheduler`] decides which schedules are due; it is checked on the
//! agent's periodic tick with the agent's clock and the computer's time
//! zone. Lifecycle events are found by comparing the agent's state after
//! every event it handles with the state seen before: the active profile,
//! its active project and the connected devices.
//!
//! - Triggers fire only for the **active profile** and only while the
//!   configuration is **unlocked**. Nothing that happened while locked is
//!   replayed: unlocking takes the current state as the new baseline.
//! - `agentStarted` fires once per agent start, when the configuration is
//!   unlocked for the first time.
//! - A firing is **skipped** while the previous run of that workflow is
//!   still running, when the workflow's triggers are paused, and when the
//!   agent already runs as many workflows as it may.
//! - A trigger's inputs come from fixed values and the event's fields (see
//!   [`deckpilot_core::trigger`]).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Timelike};
use deckpilot_core::trigger::{LocalDate, Weekday};
use deckpilot_core::{
    EventField, ExecutionTrigger, LifecycleEvent, ProfileId, ProjectId, Scheduler, TriggerEvent,
    TriggerKind, WallTime, WorkflowId,
};
use deckpilot_device::DeviceId;
use deckpilot_ipc::SecurityState;
use tracing::{debug, info, warn};

use super::Agent;

/// How a run is started, beyond its workflow and trigger kind.
#[derive(Debug, Default)]
pub(crate) struct Launch {
    /// Values for the workflow's inputs.
    pub inputs: BTreeMap<String, String>,
    /// Which schedule or event started it, for history.
    pub detail: Option<String>,
}

/// What the agent's lifecycle looked like when last observed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Observed {
    profile: ProfileId,
    project: Option<ProjectId>,
    devices: BTreeSet<DeviceId>,
}

#[derive(Debug, Default)]
pub(crate) struct Triggers {
    scheduler: Scheduler,
    /// `None` while locked or without an active profile.
    observed: Option<Observed>,
    /// `agentStarted` was handled.
    started: bool,
}

/// The scheduler's view of `now` in the computer's time zone.
pub(crate) fn wall_time(now: SystemTime) -> WallTime {
    let local: chrono::DateTime<chrono::Local> = now.into();
    WallTime {
        epoch_ms: now
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
        date: LocalDate {
            year: local.year(),
            month: u8::try_from(local.month()).unwrap_or(1),
            day: u8::try_from(local.day()).unwrap_or(1),
        },
        weekday: Weekday::from_monday(local.weekday().num_days_from_monday()),
        minute: u16::try_from(local.hour() * 60 + local.minute()).unwrap_or(0),
    }
}

impl Agent {
    fn triggers_active(&self) -> bool {
        self.security == SecurityState::Unlocked && self.runtime.profile().is_some()
    }

    /// Hook for the periodic tick: starts the scheduled workflows that are
    /// due.
    pub(super) fn tick_schedules(&mut self) {
        if !self.triggers_active() {
            self.triggers.scheduler.reset();
            return;
        }
        let now = wall_time((self.options.clock)());
        let Some(profile) = self.runtime.profile() else {
            return;
        };
        let due = self.triggers.scheduler.due(&profile.workflows, &now);
        for due in due {
            let event = self.event_fields(&now);
            self.fire(due.workflow, due.trigger, &event);
        }
    }

    /// Hook after every event the agent handles: fires lifecycle triggers
    /// for what changed since the last call.
    pub(super) fn observe_lifecycle(&mut self) {
        let current = if self.security == SecurityState::Unlocked {
            self.runtime.profile().map(|profile| Observed {
                profile: profile.id,
                project: self.runtime.active_project_id(),
                devices: self.sessions.keys().cloned().collect(),
            })
        } else {
            None
        };
        if current == self.triggers.observed {
            return;
        }
        let previous = std::mem::replace(&mut self.triggers.observed, current.clone());
        let Some(current) = current else {
            // Locked, or no profile: nothing fires; schedules start afresh.
            self.triggers.scheduler.reset();
            return;
        };
        let now = wall_time((self.options.clock)());
        let Some(previous) = previous else {
            // Unlocked (or a profile appeared): the new baseline.
            if !self.triggers.started {
                self.triggers.started = true;
                self.fire_lifecycle(LifecycleEvent::AgentStarted, &self.event_fields(&now));
            }
            return;
        };
        if previous.profile != current.profile {
            self.triggers.scheduler.reset();
            self.fire_lifecycle(LifecycleEvent::ProfileActivated, &self.event_fields(&now));
        } else if previous.project != current.project {
            let before = previous.project.and_then(|id| {
                self.runtime
                    .profile()
                    .and_then(|p| p.project(id))
                    .map(|p| p.key.to_string())
            });
            let event = self
                .event_fields(&now)
                .with(EventField::PreviousProjectKey, before);
            self.fire_lifecycle(LifecycleEvent::ProjectChanged, &event);
        }
        for (devices, kind) in [
            (
                current.devices.difference(&previous.devices),
                LifecycleEvent::DeviceConnected,
            ),
            (
                previous.devices.difference(&current.devices),
                LifecycleEvent::DeviceDisconnected,
            ),
        ] {
            for device in devices {
                let model = self
                    .runtime
                    .catalog()
                    .get(&device.model)
                    .map(|m| m.name.clone());
                let event = self
                    .event_fields(&now)
                    .with(EventField::DeviceId, Some(device.serial.clone()))
                    .with(EventField::DeviceModel, model);
                self.fire_lifecycle(kind, &event);
            }
        }
    }

    /// Fields every event has: time, profile and project.
    fn event_fields(&self, now: &WallTime) -> TriggerEvent {
        let project = self.runtime.active_project();
        TriggerEvent::default()
            .with(EventField::FiredAt, Some(now.local_text()))
            .with(
                EventField::ProfileName,
                self.runtime.profile().map(|p| p.name.clone()),
            )
            .with(EventField::ProjectKey, project.map(|p| p.key.to_string()))
            .with(EventField::ProjectName, project.map(|p| p.name.clone()))
    }

    fn fire_lifecycle(&mut self, kind: LifecycleEvent, event: &TriggerEvent) {
        let Some(profile) = self.runtime.profile() else {
            return;
        };
        let matching: Vec<(WorkflowId, usize)> = profile
            .workflows
            .iter()
            .flat_map(|workflow| {
                workflow
                    .triggers
                    .iter()
                    .enumerate()
                    .filter(move |(_, t)| t.kind == TriggerKind::Lifecycle { event: kind })
                    .map(move |(index, _)| (workflow.id, index))
            })
            .collect();
        for (workflow, index) in matching {
            self.fire(workflow, index, event);
        }
    }

    /// Starts `workflow` for its trigger `index`, unless it is paused or
    /// already running.
    fn fire(&mut self, workflow: WorkflowId, index: usize, event: &TriggerEvent) {
        let Some(profile) = self.runtime.profile() else {
            return;
        };
        let Some(definition) = profile.workflow(workflow) else {
            return;
        };
        let Some(trigger) = definition.triggers.get(index) else {
            return;
        };
        let detail = trigger.kind.to_string();
        if definition.triggers_paused {
            debug!(workflow = %definition.name, %detail, "trigger paused");
            return;
        }
        if self.is_running(workflow) {
            info!(workflow = %definition.name, %detail, "skipped: the previous run is still running");
            return;
        }
        let kind = match trigger.kind {
            TriggerKind::Schedule { .. } => ExecutionTrigger::Schedule,
            TriggerKind::Lifecycle { .. } => ExecutionTrigger::Lifecycle,
        };
        let inputs = match definition.trigger_inputs(trigger, event, profile) {
            Ok(inputs) => inputs,
            Err(error) => {
                warn!(workflow = %definition.name, %detail, %error, "trigger cannot start the workflow");
                return;
            }
        };
        let name = definition.name.clone();
        let launch = Launch {
            inputs,
            detail: Some(detail.clone()),
        };
        match self.start_execution_with(workflow, kind, launch) {
            Ok(_) => info!(workflow = %name, %detail, "workflow started by its trigger"),
            Err(error) => {
                warn!(workflow = %name, %detail, %error, "trigger cannot start the workflow");
            }
        }
    }
}

/// Daily and weekly schedules through real daylight saving time changes:
/// the scheduler is ticked every 5 s (the agent's check interval) with
/// [`wall_time`] in an actual time zone. The zone is process-wide (`TZ`),
/// so each test re-runs itself in a child process with `TZ` set.
#[cfg(all(test, unix))]
mod dst_tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use deckpilot_core::trigger::{Schedule, Weekday, WorkflowTrigger};
    use deckpilot_core::{Scheduler, Workflow};

    use super::wall_time;

    const CHILD: &str = "DECKPILOT_DST_TEST_CHILD";
    const TICK_SECONDS: u64 = 5;
    const HOUR: u64 = 3600;

    /// Runs `check` with `TZ=zone` in a child process; the test named
    /// `name` is the caller itself.
    fn in_zone(zone: &str, name: &str, check: impl FnOnce()) {
        if std::env::var_os(CHILD).is_some() {
            check();
            return;
        }
        if !std::path::Path::new("/usr/share/zoneinfo")
            .join(zone)
            .exists()
        {
            eprintln!("skipped: no time zone data for {zone}");
            return;
        }
        let module = module_path!().split_once("::").map_or("", |(_, rest)| rest);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([&format!("{module}::{name}"), "--exact", "--test-threads=1"])
            .env(CHILD, "1")
            .env("TZ", zone)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{zone}:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("1 passed"),
            "the child did not run the test:\n{stdout}"
        );
    }

    fn system_time(unix_seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(unix_seconds)
    }

    /// Firings per `(schedule label, local date)` while ticking through
    /// `hours` from `start` (Unix seconds). The scheduler already runs for
    /// an hour before `start` (uncounted), because a time that is due when
    /// a schedule is first seen does not fire.
    fn simulate(
        start: u64,
        hours: u64,
        labelled: &[(&str, Schedule)],
    ) -> BTreeMap<(String, String), u32> {
        let workflows: Vec<Workflow> = labelled
            .iter()
            .map(|(label, schedule)| {
                let mut workflow = Workflow::new(*label);
                workflow
                    .triggers
                    .push(WorkflowTrigger::schedule(schedule.clone()));
                workflow
            })
            .collect();
        let mut scheduler = Scheduler::default();
        let mut fired = BTreeMap::new();
        let first = start - HOUR;
        for tick in 0..(hours + 1) * HOUR / TICK_SECONDS {
            let seconds = first + tick * TICK_SECONDS;
            let now = wall_time(system_time(seconds));
            let due = scheduler.due(&workflows, &now);
            if seconds < start {
                continue;
            }
            for due in due {
                let label = workflows
                    .iter()
                    .find(|w| w.id == due.workflow)
                    .map(|w| w.name.clone())
                    .unwrap();
                *fired
                    .entry((label, now.local_text()[..10].to_owned()))
                    .or_insert(0) += 1;
            }
        }
        fired
    }

    fn daily(times: &[&'static str]) -> Vec<(&'static str, Schedule)> {
        times
            .iter()
            .map(|at| {
                (
                    *at,
                    Schedule::Daily {
                        at: (*at).to_owned(),
                    },
                )
            })
            .collect()
    }

    /// `fired` holds exactly `expected`: once for each listed date, never
    /// twice, and nothing on other dates.
    fn assert_fired(fired: &BTreeMap<(String, String), u32>, expected: &[(&str, &str)]) {
        let expected: BTreeMap<(String, String), u32> = expected
            .iter()
            .map(|(label, date)| (((*label).to_owned(), (*date).to_owned()), 1))
            .collect();
        assert_eq!(fired, &expected);
    }

    /// The child really runs in the zone: `unix_seconds` shows as `local`.
    fn assert_local(unix_seconds: u64, local: &str) {
        assert_eq!(wall_time(system_time(unix_seconds)).local_text(), local);
    }

    fn on(times: &[&'static str], date: &'static str) -> Vec<(&'static str, &'static str)> {
        times.iter().map(|t| (*t, date)).collect()
    }

    #[test]
    fn berlin_spring_forward_skips_the_missing_hour_for_one_day_only() {
        in_zone(
            "Europe/Berlin",
            "berlin_spring_forward_skips_the_missing_hour_for_one_day_only",
            || {
                // 2026-03-29 02:00 CET becomes 03:00 CEST (01:00 UTC).
                assert_local(1_774_745_940, "2026-03-29 01:59");
                assert_local(1_774_746_000, "2026-03-29 03:00");
                let times = ["01:30", "02:00", "02:30", "03:00", "03:30", "09:00"];
                // From 2026-03-29 00:00 CET for 47 h: all of the 29th (23 h
                // long) and the 30th.
                let fired = simulate(1_774_738_800, 47, &daily(&times));
                let mut expected = on(&["01:30", "03:00", "03:30", "09:00"], "2026-03-29");
                expected.extend(on(&times, "2026-03-30"));
                assert_fired(&fired, &expected);
            },
        );
    }

    #[test]
    fn berlin_fall_back_fires_a_repeated_time_once() {
        in_zone(
            "Europe/Berlin",
            "berlin_fall_back_fires_a_repeated_time_once",
            || {
                // 2026-10-25 03:00 CEST becomes 02:00 CET (01:00 UTC): 02:00
                // to 02:59 are shown twice.
                assert_local(1_792_889_940, "2026-10-25 02:59");
                assert_local(1_792_890_000, "2026-10-25 02:00");
                let times = ["01:30", "02:00", "02:30", "02:59", "03:00", "09:00"];
                // From 2026-10-25 00:00 CEST for 49 h: the 25th (25 h long)
                // and the 26th.
                let fired = simulate(1_792_879_200, 49, &daily(&times));
                let mut expected = on(&times, "2026-10-25");
                expected.extend(on(&times, "2026-10-26"));
                assert_fired(&fired, &expected);
            },
        );
    }

    #[test]
    fn new_york_both_changes() {
        in_zone("America/New_York", "new_york_both_changes", || {
            // 2026-03-08 02:00 EST becomes 03:00 EDT (07:00 UTC).
            assert_local(1_772_953_140, "2026-03-08 01:59");
            assert_local(1_772_953_200, "2026-03-08 03:00");
            let times = ["01:30", "02:15", "03:00", "23:59"];
            // From 2026-03-08 00:00 EST for 47 h.
            let fired = simulate(1_772_946_000, 47, &daily(&times));
            let mut expected = on(&["01:30", "03:00", "23:59"], "2026-03-08");
            expected.extend(on(&times, "2026-03-09"));
            assert_fired(&fired, &expected);
            // 2026-11-01 02:00 EDT becomes 01:00 EST (06:00 UTC).
            assert_local(1_793_512_740, "2026-11-01 01:59");
            assert_local(1_793_512_800, "2026-11-01 01:00");
            let times = ["00:30", "01:00", "01:30", "02:00"];
            // From 2026-11-01 00:00 EDT for 49 h.
            let fired = simulate(1_793_505_600, 49, &daily(&times));
            let mut expected = on(&times, "2026-11-01");
            expected.extend(on(&times, "2026-11-02"));
            assert_fired(&fired, &expected);
        });
    }

    #[test]
    fn santiago_changes_at_midnight() {
        in_zone("America/Santiago", "santiago_changes_at_midnight", || {
            // 2026-04-04 24:00 -03 becomes 23:00 -04 the same date (03:00
            // UTC): 23:00 to 23:59 are shown twice.
            assert_local(1_775_357_940, "2026-04-04 23:59");
            assert_local(1_775_358_000, "2026-04-04 23:00");
            let times = ["00:30", "23:00", "23:30"];
            // From 2026-04-04 00:00 -03 for 49 h.
            let fired = simulate(1_775_271_600, 49, &daily(&times));
            let mut expected = on(&times, "2026-04-04");
            expected.extend(on(&times, "2026-04-05"));
            assert_fired(&fired, &expected);
            // 2026-09-06 00:00 -04 becomes 01:00 -03 (04:00 UTC): the
            // first hour of the 6th does not exist.
            assert_local(1_788_667_140, "2026-09-05 23:59");
            assert_local(1_788_667_200, "2026-09-06 01:00");
            let times = ["00:00", "00:30", "01:00", "23:30"];
            // From 2026-09-05 00:00 -04 for 47 h: the 5th, and the 6th
            // (23 h long).
            let fired = simulate(1_788_580_800, 47, &daily(&times));
            let mut expected = on(&times, "2026-09-05");
            expected.extend(on(&["01:00", "23:30"], "2026-09-06"));
            assert_fired(&fired, &expected);
        });
    }

    #[test]
    fn lord_howe_half_hour_changes() {
        in_zone("Australia/Lord_Howe", "lord_howe_half_hour_changes", || {
            // 2026-04-05 02:00 +11 becomes 01:30 +10:30: 01:30 to 01:59
            // are shown twice.
            assert_local(1_775_314_740, "2026-04-05 01:59");
            assert_local(1_775_314_800, "2026-04-05 01:30");
            let times = ["01:15", "01:30", "01:45", "02:00"];
            // From 2026-04-05 00:00 +11 for 48 h (the 5th is 24.5 h long).
            let fired = simulate(1_775_307_600, 48, &daily(&times));
            let mut expected = on(&times, "2026-04-05");
            expected.extend(on(&times, "2026-04-06"));
            assert_fired(&fired, &expected);
            // 2026-10-04 02:00 +10:30 becomes 02:30 +11.
            assert_local(1_791_041_340, "2026-10-04 01:59");
            assert_local(1_791_041_400, "2026-10-04 02:30");
            let times = ["01:59", "02:00", "02:15", "02:30"];
            // From 2026-10-04 00:00 +10:30 for 47 h.
            let fired = simulate(1_791_034_200, 47, &daily(&times));
            let mut expected = on(&["01:59", "02:30"], "2026-10-04");
            expected.extend(on(&times, "2026-10-05"));
            assert_fired(&fired, &expected);
        });
    }

    #[test]
    fn weekly_time_skipped_by_spring_forward_fires_the_next_week() {
        in_zone(
            "Europe/Berlin",
            "weekly_time_skipped_by_spring_forward_fires_the_next_week",
            || {
                let schedules = [(
                    "sun 02:30",
                    Schedule::Weekly {
                        days: vec![Weekday::Sun],
                        at: "02:30".into(),
                    },
                )];
                // From Saturday 2026-03-28 00:00 CET for 9 days.
                let fired = simulate(1_774_652_400, 9 * 24, &schedules);
                assert_fired(&fired, &[("sun 02:30", "2026-04-05")]);
            },
        );
    }
}
