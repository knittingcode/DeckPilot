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
