//! Workflow triggers: schedules and lifecycle events that start a workflow
//! in the agent without anyone pressing a key.
//!
//! A workflow lists its triggers ([`Workflow::triggers`]) and can pause all
//! of them ([`Workflow::triggers_paused`]). The agent runs one [`Scheduler`]
//! and watches its own state for [`LifecycleEvent`]s; triggers fire only for
//! the active profile and only while the configuration is unlocked.
//!
//! # Schedules
//!
//! - **Interval** (`every n minutes`, at least one): due every `n` minutes
//!   after the trigger was first seen (the agent started, the profile was
//!   activated, the workflow was saved or resumed).
//! - **Daily / weekly** at a local time (`HH:MM`): due when the local wall
//!   clock shows that minute, on the chosen weekdays.
//! - **No catch-up.** A firing happens at most [`SCHEDULE_GRACE_MS`] late
//!   (the agent checks every few seconds); one that is later — the computer
//!   slept, the agent was not running or the configuration was locked — is
//!   skipped, never made up, and an interval continues with its next due
//!   time. A daily or weekly time fires at most once per local date.
//! - **Daylight saving time.** Times are local wall-clock times. A time that
//!   does not exist on a day (skipped when clocks go forward) does not fire
//!   that day; a time that occurs twice (clocks go back) fires once.
//!   Intervals count real time and are not affected.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::graph::{GraphIssue, IssueSeverity, ValueKind};
use crate::{ActionRegistry, Profile, ProjectScope, Workflow, WorkflowId};

/// Shortest interval of a schedule.
pub const MIN_INTERVAL_MINUTES: u32 = 1;
/// Longest interval of a schedule: one week.
pub const MAX_INTERVAL_MINUTES: u32 = 7 * 24 * 60;
/// Triggers a workflow can have.
pub const MAX_TRIGGERS: usize = 16;
/// How late a scheduled firing may still happen; later ones are skipped.
pub const SCHEDULE_GRACE_MS: u64 = 2 * 60 * 1000;

/// Something that starts a workflow by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTrigger {
    #[serde(flatten)]
    pub kind: TriggerKind,
    /// Values for the workflow's inputs, by input key. Inputs without a
    /// value get their default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, TriggerValue>,
}

impl WorkflowTrigger {
    pub fn schedule(schedule: Schedule) -> Self {
        Self {
            kind: TriggerKind::Schedule { schedule },
            inputs: BTreeMap::new(),
        }
    }

    pub fn lifecycle(event: LifecycleEvent) -> Self {
        Self {
            kind: TriggerKind::Lifecycle { event },
            inputs: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_input(mut self, key: impl Into<String>, value: TriggerValue) -> Self {
        self.inputs.insert(key.into(), value);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TriggerKind {
    Schedule { schedule: Schedule },
    Lifecycle { event: LifecycleEvent },
}

impl fmt::Display for TriggerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schedule { schedule } => schedule.fmt(f),
            Self::Lifecycle { event } => f.write_str(event.as_str()),
        }
    }
}

/// When a scheduled trigger is due.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "every", rename_all = "camelCase")]
pub enum Schedule {
    /// Every `minutes` minutes.
    Interval { minutes: u32 },
    /// Every day at a local time, `HH:MM`.
    Daily { at: String },
    /// On the given weekdays at a local time, `HH:MM`.
    Weekly { days: Vec<Weekday>, at: String },
}

impl fmt::Display for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interval { minutes } => write!(f, "every {minutes} min"),
            Self::Daily { at } => write!(f, "daily at {at}"),
            Self::Weekly { days, at } => {
                let days: Vec<&str> = days.iter().map(|d| d.as_str()).collect();
                write!(f, "{} at {at}", days.join(", "))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mon => "mon",
            Self::Tue => "tue",
            Self::Wed => "wed",
            Self::Thu => "thu",
            Self::Fri => "fri",
            Self::Sat => "sat",
            Self::Sun => "sun",
        }
    }

    /// Monday is 0.
    pub fn from_monday(days: u32) -> Self {
        match days % 7 {
            0 => Self::Mon,
            1 => Self::Tue,
            2 => Self::Wed,
            3 => Self::Thu,
            4 => Self::Fri,
            5 => Self::Sat,
            _ => Self::Sun,
        }
    }
}

/// A change in the agent that starts workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LifecycleEvent {
    /// The agent started: once per agent start, when the configuration is
    /// first unlocked.
    AgentStarted,
    /// Another profile became the active one.
    ProfileActivated,
    /// The active project changed within the active profile.
    ProjectChanged,
    /// A Stream Deck was connected.
    DeviceConnected,
    /// A Stream Deck was disconnected.
    DeviceDisconnected,
}

impl LifecycleEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentStarted => "agentStarted",
            Self::ProfileActivated => "profileActivated",
            Self::ProjectChanged => "projectChanged",
            Self::DeviceConnected => "deviceConnected",
            Self::DeviceDisconnected => "deviceDisconnected",
        }
    }
}

/// A value for a workflow input when a trigger starts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TriggerValue {
    /// Always this text, converted to the input's kind.
    Fixed { value: String },
    /// A field of the event that fired.
    Event { field: EventField },
}

/// What a firing knows, offered to workflow inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventField {
    /// Local date and time of the firing, `YYYY-MM-DD HH:MM`.
    FiredAt,
    /// Name of the active profile.
    ProfileName,
    /// Key of the active project, when there is one.
    ProjectKey,
    /// Name of the active project, when there is one.
    ProjectName,
    /// The project before the change (project changed).
    PreviousProjectKey,
    /// Serial of the device (device connected or disconnected).
    DeviceId,
    /// Model name of the device (device connected or disconnected).
    DeviceModel,
}

impl EventField {
    pub const ALL: [Self; 7] = [
        Self::FiredAt,
        Self::ProfileName,
        Self::ProjectKey,
        Self::ProjectName,
        Self::PreviousProjectKey,
        Self::DeviceId,
        Self::DeviceModel,
    ];

    /// The kind of value the field holds.
    pub fn kind(self) -> ValueKind {
        match self {
            Self::ProjectKey | Self::PreviousProjectKey => ValueKind::ProjectKey,
            _ => ValueKind::Text,
        }
    }

    /// Whether events of `trigger` carry the field.
    pub fn available(self, trigger: &TriggerKind) -> bool {
        let event = match trigger {
            TriggerKind::Schedule { .. } => None,
            TriggerKind::Lifecycle { event } => Some(*event),
        };
        match self {
            Self::FiredAt | Self::ProfileName | Self::ProjectKey | Self::ProjectName => true,
            Self::PreviousProjectKey => event == Some(LifecycleEvent::ProjectChanged),
            Self::DeviceId | Self::DeviceModel => matches!(
                event,
                Some(LifecycleEvent::DeviceConnected | LifecycleEvent::DeviceDisconnected)
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FiredAt => "firedAt",
            Self::ProfileName => "profileName",
            Self::ProjectKey => "projectKey",
            Self::ProjectName => "projectName",
            Self::PreviousProjectKey => "previousProjectKey",
            Self::DeviceId => "deviceId",
            Self::DeviceModel => "deviceModel",
        }
    }
}

/// The fields of one firing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TriggerEvent {
    pub fields: BTreeMap<EventField, String>,
}

impl TriggerEvent {
    #[must_use]
    pub fn with(mut self, field: EventField, value: Option<impl Into<String>>) -> Self {
        if let Some(value) = value {
            self.fields.insert(field, value.into());
        }
        self
    }
}

/// Whether a value of `from` fits an input of `to`: text inputs take
/// anything, other inputs the same kind.
fn fits(from: ValueKind, to: ValueKind) -> bool {
    to == ValueKind::Text || from == to
}

/// `HH:MM` as minutes after midnight.
pub fn parse_time(at: &str) -> Option<u16> {
    let (hours, minutes) = at.split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    let hours: u16 = hours.parse().ok()?;
    let minutes: u16 = minutes.parse().ok()?;
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

impl Workflow {
    /// Problems of the workflow's triggers, as graph-level issues (no node).
    pub fn trigger_issues(&self, profile: &Profile, registry: &ActionRegistry) -> Vec<GraphIssue> {
        let mut issues = Vec::new();
        let mut error = |message: String| {
            issues.push(GraphIssue {
                node: None,
                severity: IssueSeverity::Error,
                message,
            });
        };
        if self.triggers.len() > MAX_TRIGGERS {
            error(format!(
                "a workflow can have at most {MAX_TRIGGERS} triggers"
            ));
        }
        let changes_project =
            self.effective_project_scope(profile, registry) == ProjectScope::Application;
        for (index, trigger) in self.triggers.iter().enumerate() {
            let name = format!("trigger {} ({})", index + 1, trigger.kind);
            match &trigger.kind {
                TriggerKind::Schedule { schedule } => match schedule {
                    Schedule::Interval { minutes } => {
                        if !(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(minutes) {
                            error(format!(
                                "{name}: the interval must be between {MIN_INTERVAL_MINUTES} minute and 7 days"
                            ));
                        }
                    }
                    Schedule::Daily { at } | Schedule::Weekly { at, .. } => {
                        if parse_time(at).is_none() {
                            error(format!("{name}: `{at}` is not a time; use HH:MM"));
                        }
                        if let Schedule::Weekly { days, .. } = schedule {
                            let mut unique = days.clone();
                            unique.sort();
                            unique.dedup();
                            if days.is_empty() || unique.len() != days.len() {
                                error(format!(
                                    "{name}: choose each weekday at most once, and at least one"
                                ));
                            }
                        }
                    }
                },
                TriggerKind::Lifecycle { event } => {
                    if *event == LifecycleEvent::ProjectChanged && changes_project {
                        error(format!(
                            "{name}: a workflow started by project changes may not change the active project; it would start itself again"
                        ));
                    }
                }
            }
            for (key, value) in &trigger.inputs {
                let Some(input) = self.inputs.iter().find(|i| &i.key == key) else {
                    error(format!("{name}: `{key}` is not an input of this workflow"));
                    continue;
                };
                match value {
                    TriggerValue::Fixed { value } => {
                        if let Err(problem) =
                            crate::graph::mapping::convert(input.kind, value, profile)
                        {
                            error(format!("{name}: the input `{key}` {problem}"));
                        }
                    }
                    TriggerValue::Event { field } => {
                        if !field.available(&trigger.kind) {
                            error(format!(
                                "{name}: `{}` is not known for this trigger",
                                field.as_str()
                            ));
                        } else if !fits(field.kind(), input.kind) {
                            error(format!(
                                "{name}: `{}` does not fit the input `{key}`",
                                field.as_str()
                            ));
                        }
                    }
                }
            }
            for input in &self.inputs {
                if input.required && !trigger.inputs.contains_key(&input.key) {
                    error(format!(
                        "{name}: the required input `{}` needs a value",
                        input.key
                    ));
                }
            }
        }
        issues
    }

    /// The input values `trigger` gives the workflow for `event`, converted
    /// to their kinds. An event field the event does not carry leaves the
    /// input to its default; a required input then makes the firing fail.
    pub fn trigger_inputs(
        &self,
        trigger: &WorkflowTrigger,
        event: &TriggerEvent,
        profile: &Profile,
    ) -> Result<BTreeMap<String, String>, String> {
        let mut values = BTreeMap::new();
        for (key, value) in &trigger.inputs {
            let Some(input) = self.inputs.iter().find(|i| &i.key == key) else {
                return Err(format!("`{key}` is not an input of {}", self.name));
            };
            let text = match value {
                TriggerValue::Fixed { value } => value.clone(),
                TriggerValue::Event { field } => match event.fields.get(field) {
                    Some(value) => value.clone(),
                    None if input.required => {
                        return Err(format!(
                            "the input `{key}` needs `{}`, which this event does not have",
                            field.as_str()
                        ));
                    }
                    None => continue,
                },
            };
            let value = crate::graph::mapping::convert(input.kind, &text, profile)
                .map_err(|e| format!("the input `{key}` {e}"))?;
            values.insert(key.clone(), value);
        }
        Ok(values)
    }
}

/// A local calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

/// A moment as the scheduler sees it: real time for intervals and the
/// local wall clock for daily and weekly times. The agent derives it from
/// its clock and the computer's time zone; tests make it up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallTime {
    /// Milliseconds since the Unix epoch.
    pub epoch_ms: u64,
    pub date: LocalDate,
    pub weekday: Weekday,
    /// Local minutes after midnight.
    pub minute: u16,
}

impl WallTime {
    /// `YYYY-MM-DD HH:MM`, for the `firedAt` field.
    pub fn local_text(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            self.date.year,
            self.date.month,
            self.date.day,
            self.minute / 60,
            self.minute % 60
        )
    }
}

/// A scheduled trigger that is due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub workflow: WorkflowId,
    /// Index into the workflow's triggers.
    pub trigger: usize,
}

#[derive(Debug, Clone)]
struct Slot {
    workflow: WorkflowId,
    trigger: usize,
    schedule: Schedule,
    /// Interval: when it is due next.
    next_ms: u64,
    /// Daily, weekly: the local date it fired (or was skipped) last.
    done_on: Option<LocalDate>,
}

/// Decides which scheduled triggers are due. Holds no clock: the caller
/// passes the time, so it can be tested without waiting.
#[derive(Debug, Default)]
pub struct Scheduler {
    slots: Vec<Slot>,
}

impl Scheduler {
    /// Forgets every schedule, e.g. when the configuration is locked or
    /// another profile becomes active. They start afresh when seen again.
    pub fn reset(&mut self) {
        self.slots.clear();
    }

    /// The scheduled triggers of `workflows` that are due at `now`. Paused
    /// workflows and removed or changed triggers are forgotten; new ones
    /// start counting at `now` and never fire right away.
    pub fn due(&mut self, workflows: &[Workflow], now: &WallTime) -> Vec<Due> {
        let mut slots = Vec::new();
        let mut due = Vec::new();
        for workflow in workflows.iter().filter(|w| !w.triggers_paused) {
            for (index, trigger) in workflow.triggers.iter().enumerate() {
                let TriggerKind::Schedule { schedule } = &trigger.kind else {
                    continue;
                };
                let known = self.slots.iter().position(|s| {
                    s.workflow == workflow.id && s.trigger == index && &s.schedule == schedule
                });
                let mut slot = match known {
                    Some(position) => self.slots.swap_remove(position),
                    None => new_slot(workflow.id, index, schedule, now),
                };
                if check(&mut slot, now) {
                    due.push(Due {
                        workflow: workflow.id,
                        trigger: index,
                    });
                }
                slots.push(slot);
            }
        }
        self.slots = slots;
        due
    }
}

fn interval_ms(minutes: u32) -> u64 {
    u64::from(minutes.max(MIN_INTERVAL_MINUTES)) * 60_000
}

fn new_slot(workflow: WorkflowId, trigger: usize, schedule: &Schedule, now: &WallTime) -> Slot {
    let mut slot = Slot {
        workflow,
        trigger,
        schedule: schedule.clone(),
        next_ms: 0,
        done_on: None,
    };
    match schedule {
        Schedule::Interval { minutes } => slot.next_ms = now.epoch_ms + interval_ms(*minutes),
        // A time that is due right now when the trigger is first seen
        // (saved, the agent started) waits for the next day: no catch-up.
        Schedule::Daily { .. } | Schedule::Weekly { .. } => {
            if in_window(schedule, now) {
                slot.done_on = Some(now.date);
            }
        }
    }
    slot
}

/// Whether a daily or weekly schedule is in its firing window at `now`.
fn in_window(schedule: &Schedule, now: &WallTime) -> bool {
    let (at, days) = match schedule {
        Schedule::Interval { .. } => return false,
        Schedule::Daily { at } => (at, None),
        Schedule::Weekly { days, at } => (at, Some(days)),
    };
    let Some(at) = parse_time(at) else {
        return false;
    };
    let grace = u16::try_from(SCHEDULE_GRACE_MS / 60_000).unwrap_or(1);
    days.is_none_or(|days| days.contains(&now.weekday))
        && now.minute >= at
        && now.minute < at + grace
}

/// Whether `slot` fires at `now`, and moves it on.
fn check(slot: &mut Slot, now: &WallTime) -> bool {
    match &slot.schedule {
        Schedule::Interval { minutes } => {
            let interval = interval_ms(*minutes);
            if now.epoch_ms + interval < slot.next_ms {
                // The clock went back: start counting again.
                slot.next_ms = now.epoch_ms + interval;
                return false;
            }
            if now.epoch_ms < slot.next_ms {
                return false;
            }
            // The latest due time that passed; earlier ones are not made up.
            let missed = (now.epoch_ms - slot.next_ms) / interval;
            let last_due = slot.next_ms + missed * interval;
            slot.next_ms = last_due + interval;
            now.epoch_ms - last_due <= SCHEDULE_GRACE_MS
        }
        schedule => {
            if slot.done_on == Some(now.date) || !in_window(schedule, now) {
                return false;
            }
            slot.done_on = Some(now.date);
            true
        }
    }
}

#[cfg(test)]
mod tests;
