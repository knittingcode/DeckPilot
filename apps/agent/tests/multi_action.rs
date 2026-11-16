//! Multi Actions against a real agent with a simulated MK.2 and temporary
//! data: a key bound to a step workflow whose steps are separated by delay
//! rows (`general.delay` steps with their own duration) runs them in order
//! in the workflow engine, can be cancelled in the middle of a delay, and is
//! stored as the ordinary step list earlier versions read. A Stream Deck
//! plugged into the machine is never opened.

mod support;

use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, ExecutionState,
    ExecutionSummary, OnFailure, ParameterValue, Profile, Trigger, Workflow, WorkflowStep,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::{ConnectionState, ControlId, models};
use deckpilot_ipc::{AgentClient, AgentEvent, Request};
use deckpilot_modules::general;
use deckpilot_storage::ConfigStore;
use support::{TIMEOUT, TempDir, TestAgent};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

/// A Multi Action: log, a delay of `delay_ms`, log. The delay is the
/// profile's one Delay action with the step's own duration.
fn profile(delay_ms: i64) -> (Profile, Workflow) {
    let registry = support::registry();
    let mut profile = Profile::new("Work");
    let first = registry
        .create_instance(
            "First",
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text("first".into())),
        )
        .unwrap();
    let last = registry
        .create_instance(
            "Last",
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text("last".into())),
        )
        .unwrap();
    let delay = registry
        .create_instance(
            "Delay",
            general::delay_id(),
            &ActionParameters::new().with("milliseconds", ParameterValue::Integer(500)),
        )
        .unwrap();
    let mut wait = WorkflowStep::new(delay.id, OnFailure::Stop);
    wait.overrides =
        ActionParameters::new().with("milliseconds", ParameterValue::Integer(delay_ms));
    let mut workflow = Workflow::new("Multi Action")
        .step(first.id, OnFailure::Stop)
        .step(last.id, OnFailure::Stop);
    workflow.steps.insert(1, wait);
    profile.actions.extend([first, last, delay]);
    profile.workflows.push(workflow.clone());
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout.pages[0].bindings.push(ControlBinding::new(
        key(0),
        Trigger::Press,
        BindingTarget::Workflow {
            workflow: workflow.id,
        },
    ));
    profile.layouts.push(layout);
    (profile, workflow)
}

async fn next<T>(events: &mut AgentClient, pick: impl Fn(AgentEvent) -> Option<T>) -> T {
    loop {
        let event = tokio::time::timeout(TIMEOUT, events.next_event())
            .await
            .expect("an event in time")
            .unwrap();
        if let Some(found) = pick(event) {
            return found;
        }
    }
}

async fn started(events: &mut AgentClient) -> ExecutionSummary {
    next(events, |event| match event {
        AgentEvent::ExecutionStarted { execution } => Some(execution),
        _ => None,
    })
    .await
}

async fn finished(events: &mut AgentClient, run: &ExecutionSummary) -> ExecutionSummary {
    let id = run.id;
    next(events, move |event| match event {
        AgentEvent::ExecutionFinished { execution } if execution.id == id => Some(execution),
        _ => None,
    })
    .await
}

async fn ready(agent: &TestAgent, device: &deckpilot_device::DeviceId) {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let status = agent.status().await;
        if status
            .devices
            .iter()
            .any(|d| d.id == *device && d.state == ConnectionState::Connected && d.page.is_some())
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() <= deadline,
            "the device connects"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_runs_its_steps_in_order_with_the_delay_between_them() {
    let dir = TempDir::new("deckpilot-multi-action");
    let (work, workflow) = profile(300);
    let (paths, db_key) = support::data_dir(&dir, &work, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    ready(&agent, &device).await;
    let mut events = agent.events().await;

    let begun = Instant::now();
    mock.tap(&device, key(0));
    let run = started(&mut events).await;
    assert_eq!(run.workflow, workflow.id);
    let done = finished(&mut events, &run).await;
    assert_eq!(done.state, ExecutionState::Succeeded, "{done:?}");
    assert_eq!(done.completed_nodes, 3);
    // The delay row waited its own 300 ms, not the action's 500 ms.
    let took = begun.elapsed();
    assert!(took >= Duration::from_millis(300), "{took:?}");
    agent.shutdown().await;

    // Stored as the plain step list it was saved as.
    let stored = support::open_store(&paths, &db_key)
        .load_profile(work.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.workflows, vec![workflow]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multi_action_stops_in_the_middle_of_a_delay_when_cancelled() {
    let dir = TempDir::new("deckpilot-multi-action-cancel");
    let (work, _) = profile(general::MAX_DELAY_MS);
    let (paths, db_key) = support::data_dir(&dir, &work, &[]);
    let mock = MockBackend::new();
    let device = mock.plug(&models::stream_deck_mk2(), "MK2-A");
    let agent = TestAgent::start_unlocked(&paths, &db_key, &mock, None).await;
    ready(&agent, &device).await;
    let mut events = agent.events().await;

    mock.tap(&device, key(0));
    let run = started(&mut events).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let begun = Instant::now();
    agent
        .request(Request::CancelExecution { execution: run.id })
        .await
        .unwrap();
    let done = finished(&mut events, &run).await;
    assert_eq!(done.state, ExecutionState::Cancelled, "{done:?}");
    // The five-minute delay ended at once.
    assert!(begun.elapsed() < Duration::from_secs(5));
    let ran = done.finished_at.unwrap() - done.started_at;
    assert!(ran < 5_000, "ran {ran} ms");
    agent.shutdown().await;
}

/// A step workflow as 0.6.0-alpha.3 stores it reads and writes back
/// unchanged: a Multi Action is no new format.
#[test]
fn a_step_workflow_from_alpha3_round_trips_unchanged() {
    let alpha3 = serde_json::json!({
        "id": "3f1c2b4a-5d6e-4f70-8a9b-0c1d2e3f4a50",
        "name": "Start Work",
        "steps": [
            {
                "action": "0b6c0f2e-1f0a-4f0e-8a67-0c8f4b5a7d11",
                "onFailure": "continue",
                "condition": { "type": "always" }
            },
            {
                "action": "0b6c0f2e-1f0a-4f0e-8a67-0c8f4b5a7d12",
                "onFailure": "stop",
                "condition": { "type": "previousSucceeded" },
                "overrides": { "seconds": { "type": "integer", "value": 2 } }
            }
        ],
        "projectScope": "execution"
    });
    let read: Workflow = serde_json::from_value(alpha3.clone()).unwrap();
    assert_eq!(serde_json::to_value(&read).unwrap(), alpha3);
}
