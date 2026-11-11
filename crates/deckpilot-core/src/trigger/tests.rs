use super::*;
use crate::ExecutionTrigger;
use crate::graph::{ValueKind, WorkflowInput};
use crate::testing::{registry, with_projects};

const MINUTE: u64 = 60_000;

/// 2026-03-02 (a Monday) at `minute` past midnight, plus `extra_ms`.
fn at(day: u8, minute: u16, extra_ms: u64) -> WallTime {
    let date = LocalDate {
        year: 2026,
        month: 3,
        day,
    };
    WallTime {
        epoch_ms: 1_772_400_000_000
            + u64::from(day) * 24 * 60 * MINUTE
            + u64::from(minute) * MINUTE
            + extra_ms,
        date,
        weekday: Weekday::from_monday(u32::from(day) - 2),
        minute,
    }
}

fn scheduled(schedule: Schedule) -> Workflow {
    let mut workflow = Workflow::new("Scheduled");
    workflow.triggers.push(WorkflowTrigger::schedule(schedule));
    workflow
}

fn fires(scheduler: &mut Scheduler, workflows: &[Workflow], now: &WallTime) -> bool {
    !scheduler.due(workflows, now).is_empty()
}

#[test]
fn intervals_fire_on_time_and_never_catch_up() {
    let workflows = [scheduled(Schedule::Interval { minutes: 15 })];
    let mut scheduler = Scheduler::default();
    // Counting starts when the trigger is first seen; nothing fires then.
    assert!(!fires(&mut scheduler, &workflows, &at(2, 600, 0)));
    assert!(!fires(&mut scheduler, &workflows, &at(2, 614, 59_000)));
    let due = scheduler.due(&workflows, &at(2, 615, 3_000));
    assert_eq!(
        due,
        [Due {
            workflow: workflows[0].id,
            trigger: 0
        }]
    );
    // Once per due time, however often it is checked.
    assert!(!fires(&mut scheduler, &workflows, &at(2, 615, 8_000)));
    assert!(fires(&mut scheduler, &workflows, &at(2, 630, 0)));
    // The computer slept for hours: the missed runs are not made up, and
    // the late wake-up does not fire either...
    assert!(!fires(&mut scheduler, &workflows, &at(2, 800, 0)));
    // ...it continues on its grid.
    assert!(!fires(&mut scheduler, &workflows, &at(2, 804, 0)));
    assert!(fires(&mut scheduler, &workflows, &at(2, 810, 1_000)));
    // Woken up within the grace period, it still fires.
    assert!(fires(&mut scheduler, &workflows, &at(2, 826, 30_000)));
}

#[test]
fn intervals_start_again_when_the_clock_goes_back() {
    let workflows = [scheduled(Schedule::Interval { minutes: 5 })];
    let mut scheduler = Scheduler::default();
    assert!(!fires(&mut scheduler, &workflows, &at(2, 600, 0)));
    assert!(!fires(&mut scheduler, &workflows, &at(2, 500, 0)));
    assert!(!fires(&mut scheduler, &workflows, &at(2, 504, 0)));
    assert!(fires(&mut scheduler, &workflows, &at(2, 505, 0)));
}

#[test]
fn daily_and_weekly_times_fire_once_per_date_in_local_time() {
    let daily = scheduled(Schedule::Daily { at: "09:30".into() });
    let weekly = scheduled(Schedule::Weekly {
        days: vec![Weekday::Mon, Weekday::Wed],
        at: "09:30".into(),
    });
    let workflows = [daily.clone(), weekly.clone()];
    let mut scheduler = Scheduler::default();
    assert!(!fires(&mut scheduler, &workflows, &at(2, 569, 0)));
    // Monday 09:30: both.
    assert_eq!(scheduler.due(&workflows, &at(2, 570, 0)).len(), 2);
    assert!(!fires(&mut scheduler, &workflows, &at(2, 570, 30_000)));
    // A wall clock that goes back an hour (daylight saving time ends)
    // shows 09:30 again: it does not fire twice on the same date.
    assert!(!fires(&mut scheduler, &workflows, &at(2, 570, 3_600_000)));
    // Tuesday: only the daily one.
    let due = scheduler.due(&workflows, &at(3, 571, 0));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].workflow, daily.id);
    // Wednesday, but the computer slept through 09:30: no catch-up.
    assert!(!fires(&mut scheduler, &workflows, &at(4, 600, 0)));
    // A time the clock skips (daylight saving time starts) is not seen and
    // does not fire that day; the next day it does.
    assert!(!fires(&mut scheduler, &workflows, &at(5, 569, 0)));
    assert!(!fires(&mut scheduler, &workflows, &at(5, 631, 0)));
    assert_eq!(scheduler.due(&workflows, &at(6, 570, 0)).len(), 1);
}

#[test]
fn new_paused_and_changed_triggers_start_afresh() {
    let mut workflow = scheduled(Schedule::Daily { at: "09:30".into() });
    let mut scheduler = Scheduler::default();
    // First seen inside its minute (saved at 09:30:20): waits for tomorrow.
    assert!(!fires(
        &mut scheduler,
        &[workflow.clone()],
        &at(2, 570, 20_000)
    ));
    assert!(!fires(&mut scheduler, &[workflow.clone()], &at(2, 571, 0)));
    assert!(fires(&mut scheduler, &[workflow.clone()], &at(3, 570, 0)));
    // Paused: nothing fires.
    workflow.triggers_paused = true;
    assert!(!fires(&mut scheduler, &[workflow.clone()], &at(4, 570, 0)));
    // A changed interval counts from when it is seen.
    let mut interval = scheduled(Schedule::Interval { minutes: 10 });
    assert!(!fires(&mut scheduler, &[interval.clone()], &at(5, 0, 0)));
    interval.triggers[0] = WorkflowTrigger::schedule(Schedule::Interval { minutes: 20 });
    assert!(!fires(&mut scheduler, &[interval.clone()], &at(5, 10, 0)));
    assert!(fires(&mut scheduler, &[interval.clone()], &at(5, 30, 0)));
    // Lifecycle triggers are not the scheduler's.
    let lifecycle = {
        let mut w = Workflow::new("On connect");
        w.triggers
            .push(WorkflowTrigger::lifecycle(LifecycleEvent::DeviceConnected));
        w
    };
    assert!(!fires(&mut scheduler, &[lifecycle], &at(5, 40, 0)));
    scheduler.reset();
    assert!(!fires(&mut scheduler, &[interval], &at(5, 51, 0)));
}

fn with_inputs(mut workflow: Workflow) -> Workflow {
    workflow.inputs = vec![
        WorkflowInput {
            required: true,
            ..WorkflowInput::new("serial", ValueKind::Text)
        },
        WorkflowInput {
            default: Some("2".into()),
            ..WorkflowInput::new("count", ValueKind::Integer)
        },
        WorkflowInput::new("project", ValueKind::ProjectKey),
    ];
    workflow
}

fn errors(workflow: &Workflow, profile: &Profile) -> Vec<String> {
    workflow
        .trigger_issues(profile, &registry())
        .into_iter()
        .map(|i| i.message)
        .collect()
}

#[test]
fn validation_checks_schedules_inputs_and_self_triggering() {
    let profile = with_projects("Work", &["SHOP", "CMS"]);
    let mut workflow = with_inputs(Workflow::new("Checked"));
    let event = |field| TriggerValue::Event { field };
    workflow.triggers = vec![
        WorkflowTrigger::schedule(Schedule::Interval { minutes: 0 })
            .with_input("serial", TriggerValue::Fixed { value: "x".into() }),
        WorkflowTrigger::schedule(Schedule::Daily { at: "25:00".into() })
            .with_input("serial", event(EventField::DeviceId)),
        WorkflowTrigger::schedule(Schedule::Weekly {
            days: vec![],
            at: "07:00".into(),
        })
        .with_input("serial", TriggerValue::Fixed { value: "x".into() }),
        WorkflowTrigger::lifecycle(LifecycleEvent::DeviceConnected)
            .with_input("serial", event(EventField::DeviceId))
            .with_input(
                "count",
                TriggerValue::Fixed {
                    value: "many".into(),
                },
            )
            .with_input("project", event(EventField::ProfileName))
            .with_input("nope", TriggerValue::Fixed { value: "x".into() }),
        // Missing the required input.
        WorkflowTrigger::lifecycle(LifecycleEvent::ProjectChanged),
    ];
    let errors = errors(&workflow, &profile);
    let has = |text: &str| errors.iter().any(|e| e.contains(text));
    assert!(
        has("trigger 1 (every 0 min): the interval must be between"),
        "{errors:?}"
    );
    assert!(has("`25:00` is not a time"), "{errors:?}");
    assert!(
        has("`deviceId` is not known for this trigger"),
        "{errors:?}"
    );
    assert!(has("choose each weekday"), "{errors:?}");
    assert!(
        has("the input `count` expects a whole number"),
        "{errors:?}"
    );
    assert!(
        has("`profileName` does not fit the input `project`"),
        "{errors:?}"
    );
    assert!(has("`nope` is not an input"), "{errors:?}");
    assert!(
        has("trigger 5 (projectChanged): the required input `serial` needs a value"),
        "{errors:?}"
    );
    assert!(!has("start itself again"), "{errors:?}");

    // A workflow that may change the active project cannot be started by
    // project changes: it would start itself again.
    let mut looping = Workflow::new("Loop");
    looping.project_scope = Some(ProjectScope::Application);
    looping
        .triggers
        .push(WorkflowTrigger::lifecycle(LifecycleEvent::ProjectChanged));
    assert!(
        errors_of(&looping, &profile)
            .iter()
            .any(|e| e.contains("start itself again"))
    );
    looping.project_scope = Some(ProjectScope::Execution);
    assert_eq!(errors_of(&looping, &profile), Vec::<String>::new());
    // And it reaches saving.
    looping.project_scope = Some(ProjectScope::Application);
    assert!(
        looping
            .validate(&profile, &registry())
            .iter()
            .any(|e| e.contains("start itself again"))
    );
}

fn errors_of(workflow: &Workflow, profile: &Profile) -> Vec<String> {
    errors(workflow, profile)
}

#[test]
fn inputs_come_from_fixed_values_and_event_fields() {
    let profile = with_projects("Work", &["SHOP", "CMS"]);
    let workflow = with_inputs(Workflow::new("Fed"));
    let trigger = WorkflowTrigger::lifecycle(LifecycleEvent::DeviceConnected)
        .with_input(
            "serial",
            TriggerValue::Event {
                field: EventField::DeviceId,
            },
        )
        .with_input(
            "count",
            TriggerValue::Fixed {
                value: " 7 ".into(),
            },
        )
        .with_input(
            "project",
            TriggerValue::Event {
                field: EventField::ProjectKey,
            },
        );
    let mut triggered = workflow.clone();
    triggered.triggers.push(trigger.clone());
    assert_eq!(errors(&triggered, &profile), Vec::<String>::new());
    let event = TriggerEvent::default()
        .with(EventField::DeviceId, Some("SIM-1"))
        .with(EventField::ProjectKey, Some("CMS"));
    assert_eq!(
        workflow.trigger_inputs(&trigger, &event, &profile).unwrap(),
        BTreeMap::from([
            ("count".to_owned(), "7".to_owned()),
            ("project".to_owned(), "CMS".to_owned()),
            ("serial".to_owned(), "SIM-1".to_owned()),
        ])
    );
    // No active project: the optional input is left to its default.
    let no_project = TriggerEvent::default().with(EventField::DeviceId, Some("SIM-1"));
    assert!(
        !workflow
            .trigger_inputs(&trigger, &no_project, &profile)
            .unwrap()
            .contains_key("project")
    );
    // A required input without its field cannot start the workflow.
    assert!(
        workflow
            .trigger_inputs(&trigger, &TriggerEvent::default(), &profile)
            .unwrap_err()
            .contains("`serial`")
    );
}

#[test]
fn triggers_and_trigger_kinds_serialize_compatibly() {
    let mut workflow = Workflow::new("Stored");
    workflow.triggers = vec![
        WorkflowTrigger::schedule(Schedule::Weekly {
            days: vec![Weekday::Mon],
            at: "08:00".into(),
        }),
        WorkflowTrigger::lifecycle(LifecycleEvent::DeviceConnected).with_input(
            "serial",
            TriggerValue::Event {
                field: EventField::DeviceId,
            },
        ),
    ];
    workflow.triggers_paused = true;
    let json = serde_json::to_value(&workflow).unwrap();
    assert_eq!(json["triggers"][0]["type"], "schedule");
    assert_eq!(json["triggers"][0]["schedule"]["every"], "weekly");
    assert_eq!(json["triggers"][0]["schedule"]["days"][0], "mon");
    assert_eq!(json["triggers"][1]["event"], "deviceConnected");
    assert_eq!(json["triggers"][1]["inputs"]["serial"]["field"], "deviceId");
    assert_eq!(json["triggersPaused"], true);
    assert_eq!(serde_json::from_value::<Workflow>(json).unwrap(), workflow);
    // Workflows without triggers store nothing new.
    let plain = serde_json::to_value(Workflow::new("Plain")).unwrap();
    assert!(plain.get("triggers").is_none() && plain.get("triggersPaused").is_none());

    // Execution triggers: new kinds, and kinds from a newer version read as
    // `other`, also inside a stored record.
    assert_eq!(
        serde_json::to_value(ExecutionTrigger::Schedule).unwrap(),
        "schedule"
    );
    assert_eq!(
        serde_json::from_value::<ExecutionTrigger>("webhook".into()).unwrap(),
        ExecutionTrigger::Other
    );
    let record = crate::ExecutionRecord::start(
        crate::ExecutionId::new(),
        &workflow,
        &with_projects("Work", &[]),
        ExecutionTrigger::Lifecycle,
        5,
    );
    let mut json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["trigger"], "lifecycle");
    json["trigger"] = "webhook".into();
    json["futureField"] = 1.into();
    let read: crate::ExecutionRecord = serde_json::from_value(json).unwrap();
    assert_eq!(read.summary.trigger, ExecutionTrigger::Other);
}
