//! End-to-end workflow runs in the agent: a graph that combines a
//! sub-workflow, variables, a condition, parallel branches and retries and
//! is cancelled midway; trigger-started runs against manual runs and the
//! concurrency limit; and an agent restart during a run. Simulated devices,
//! temporary data, a local mock HTTP server, nothing else.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use deckpilot_agent::Clock;
use deckpilot_core::graph::{
    NodeStatus, ValueKind, WorkflowInput, WorkflowOutput, WorkflowVariable,
};
use deckpilot_core::{
    ActionParameters, ExecutionRecord, ExecutionState, ExecutionTrigger, LifecycleEvent,
    ParameterValue, Profile, Project, ProjectKey, Schedule, Workflow, WorkflowTrigger,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_device::models;
use deckpilot_ipc::{AgentClient, AgentEvent, Request};
use deckpilot_modules::projects::ProjectCatalog;
use deckpilot_modules::{general, http};
use serde_json::json;
use support::http::{MockServer, Reply};
use support::{TIMEOUT, TempDir, TestAgent, graph, graph_workflow, waiting_workflow};

const SECRET: &str = "s3cr3t-token-e2e";

fn work_profile() -> Profile {
    let mut profile = Profile::new("Work");
    profile
        .add_project(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"))
        .unwrap();
    profile.default_project = Some(profile.projects[0].id);
    profile
}

fn node<'a>(record: &'a ExecutionRecord, id: &str) -> &'a deckpilot_core::graph::NodeReport {
    record
        .nodes
        .iter()
        .find(|n| n.node.as_str() == id)
        .unwrap_or_else(|| panic!("no node {id} in {record:#?}"))
}

/// `Verify`: takes a sensitive `secret`, returns `checked` and the secret
/// back as the sensitive `echo`.
fn verify_workflow() -> Workflow {
    let mut verify = graph_workflow(
        "Verify",
        graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "out", "type": "output", "values": {
                    "checked": {"type": "template", "template": "verified"},
                    "echo": {"type": "output", "reference": {"node": "start", "output": "secret"}},
                }},
            ]),
            &json!([{"from": "start", "to": "out"}]),
        ),
    );
    verify.inputs = vec![WorkflowInput {
        required: true,
        sensitive: true,
        ..WorkflowInput::new("secret", ValueKind::Text)
    }];
    verify.outputs = vec![
        WorkflowOutput::new("checked", ValueKind::Text),
        WorkflowOutput {
            sensitive: true,
            ..WorkflowOutput::new("echo", ValueKind::Text)
        },
    ];
    verify
}

/// `Release`: a health check that is retried, a request whose sensitive
/// value goes into a sensitive variable, a condition, then in parallel the
/// `Verify` sub-workflow (given the variable) and a long wait, a Join and a
/// last step that logs a variable.
fn release_profile(server: &MockServer) -> (Profile, Workflow) {
    let registry = support::registry();
    let mut profile = work_profile();
    let text = |value: &str| ParameterValue::Text(value.to_owned());
    let mut add = |name: &str, kind, parameters: ActionParameters| {
        let instance = registry.create_instance(name, kind, &parameters).unwrap();
        let id = instance.id;
        profile.actions.push(instance);
        id
    };
    let health = add(
        "Health",
        http::actions::id(http::actions::HEALTH_CHECK),
        ActionParameters::new().with("url", text(&server.url("/health"))),
    );
    let fetch = add(
        "Fetch token",
        http::actions::id(http::actions::REQUEST),
        ActionParameters::new()
            .with("url", text(&server.url("/token")))
            .with("extract", text("/token")),
    );
    let log = add(
        "Log",
        general::log_id(),
        ActionParameters::new().with("message", text("log")),
    );
    let verify = verify_workflow();
    let mut release = graph_workflow(
        "Release",
        graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "health", "type": "action", "action": health,
                 "policy": {"retry": {"attempts": 2, "backoffMs": 50}}},
                {"id": "fetch", "type": "action", "action": fetch},
                {"id": "remember", "type": "setVariable", "values": {
                    "token": {"type": "output", "reference": {"node": "fetch", "output": "value"}},
                    "stage": {"type": "template", "template": "fetched"},
                }},
                {"id": "healthy", "type": "condition", "predicate": {"type": "equals",
                    "reference": {"node": "health", "output": "healthy"}, "value": "true"}},
                {"id": "fork", "type": "parallel"},
                {"id": "verify", "type": "call", "workflow": verify.id, "inputs": {
                    "secret": {"type": "output", "reference": {"node": "vars", "output": "token"}},
                }},
                {"id": "verified", "type": "setVariable", "values": {
                    "stage": {"type": "template", "template": "{{ steps.verify.output.checked }}"},
                }},
                {"id": "slow", "type": "delay", "ms": 20_000},
                {"id": "join", "type": "join"},
                {"id": "finish", "type": "action", "action": log, "mappings": {
                    "message": {"type": "template", "template": "stage {{ steps.vars.output.stage }}"},
                }},
                {"id": "unhealthy", "type": "stop"},
            ]),
            &json!([
                {"from": "start", "to": "health"},
                {"from": "health", "to": "fetch"},
                {"from": "fetch", "to": "remember"},
                {"from": "remember", "to": "healthy"},
                {"from": "healthy", "port": "true", "to": "fork"},
                {"from": "healthy", "port": "false", "to": "unhealthy"},
                {"from": "fork", "to": "verify"},
                {"from": "fork", "to": "slow"},
                {"from": "verify", "to": "verified"},
                {"from": "verified", "to": "join"},
                {"from": "slow", "to": "join"},
                {"from": "join", "to": "finish"},
            ]),
        ),
    );
    release.variables = vec![
        WorkflowVariable {
            sensitive: true,
            ..WorkflowVariable::new("token", ValueKind::Text)
        },
        WorkflowVariable {
            initial: Some("start".into()),
            ..WorkflowVariable::new("stage", ValueKind::Text)
        },
    ];
    let validation = release.validate(&profile_with(&profile, &verify, &release), &registry);
    assert_eq!(validation, Vec::<String>::new());
    profile.workflows.extend([verify, release.clone()]);
    (profile, release)
}

fn profile_with(profile: &Profile, verify: &Workflow, release: &Workflow) -> Profile {
    let mut profile = profile.clone();
    profile.workflows.extend([verify.clone(), release.clone()]);
    profile
}

/// Collects the agent's events until `execution` finished.
async fn events_until_finished(
    events: &mut AgentClient,
    execution: deckpilot_core::ExecutionId,
) -> Vec<AgentEvent> {
    let mut seen = Vec::new();
    loop {
        let event = tokio::time::timeout(TIMEOUT, events.next_event())
            .await
            .expect("events in time")
            .unwrap();
        let done =
            matches!(&event, AgentEvent::ExecutionFinished { execution: e } if e.id == execution);
        seen.push(event);
        if done {
            return seen;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_combined_workflow_is_cancelled_midway_with_safe_history() {
    let server = MockServer::start();
    // Unhealthy once: the health check is retried.
    server.route(
        "/health",
        vec![Reply::text(503, "starting"), Reply::text(200, "ok")],
    );
    server.route(
        "/token",
        vec![Reply::json(200, &format!(r#"{{"token":"{SECRET}"}}"#))],
    );
    let (profile, release) = release_profile(&server);
    let dir = TempDir::new("deckpilot-e2e-combined");
    let (paths, key) = support::data_dir(
        &dir,
        &profile,
        &[(
            http::module_id(),
            ActionParameters::new().with(http::ALLOW_PRIVATE, ParameterValue::Boolean(true)),
        )],
    );
    let agent = TestAgent::start_unlocked(&paths, &key, &MockBackend::new(), None).await;
    let mut events = agent.events().await;

    let run = agent.start_workflow(release.id).await.unwrap();
    agent.wait_for_node(run.id, "slow").await;
    // The agent keeps answering while the workflow runs (and the quick
    // branch, the sub-workflow, finishes meanwhile).
    for _ in 0..5 {
        let asked = Instant::now();
        let status = tokio::time::timeout(Duration::from_secs(2), agent.status())
            .await
            .expect("status while a workflow runs");
        assert!(status.active_profile.is_some());
        assert!(asked.elapsed() < Duration::from_secs(2));
    }
    // Cancel midway: the long branch is waiting, the other one is done.
    agent
        .request(Request::CancelExecution { execution: run.id })
        .await
        .unwrap();
    let seen = events_until_finished(&mut events, run.id).await;
    let record = agent.wait_finished(run.id).await;
    assert_eq!(
        record.summary.state,
        ExecutionState::Cancelled,
        "{record:#?}"
    );

    // What ran, and how.
    let health = node(&record, "health");
    assert_eq!(health.status, NodeStatus::Succeeded);
    assert_eq!(health.attempts, 2);
    assert_eq!(
        server.requests().iter().filter(|p| *p == "/health").count(),
        2
    );
    for id in ["fetch", "remember", "healthy", "fork", "verify"] {
        assert_eq!(node(&record, id).status, NodeStatus::Succeeded, "{id}");
    }
    // Nodes run in waves: the node after the sub-workflow waits for the
    // whole wave, so the cancellation reaches it before it runs.
    for id in ["slow", "verified", "join", "finish"] {
        assert_eq!(node(&record, id).status, NodeStatus::Cancelled, "{id}");
    }
    assert_eq!(node(&record, "unhealthy").status, NodeStatus::Skipped);

    // Redaction: sensitive outputs and variables are not in the history.
    let fetch = node(&record, "fetch");
    assert_eq!(fetch.output["status"], "200");
    assert!(!fetch.output.contains_key("body") && !fetch.output.contains_key("value"));
    let remember = node(&record, "remember");
    assert_eq!(
        remember.output.get("stage").map(String::as_str),
        Some("fetched")
    );
    assert!(!remember.output.contains_key("token"));
    let verify = node(&record, "verify");
    assert_eq!(
        verify.output.get("checked").map(String::as_str),
        Some("verified")
    );
    assert!(!verify.output.contains_key("echo"));
    let stored = agent.store().execution(run.id).unwrap().unwrap();
    let stored_json = serde_json::to_string(&stored).unwrap();
    assert!(!stored_json.contains(SECRET), "{stored_json}");
    assert!(!serde_json::to_string(&record).unwrap().contains(SECRET));

    // Events: the retry was reported, nothing carried the secret, and the
    // run ended as cancelled.
    assert!(seen.iter().any(|e| matches!(
        e,
        AgentEvent::ExecutionProgress { execution, event: deckpilot_core::graph::ExecutionEvent::Retrying { .. }, .. }
            if *execution == run.id
    )));
    for event in &seen {
        let text = serde_json::to_string(event).unwrap();
        assert!(!text.contains(SECRET), "{text}");
    }
    assert!(seen.iter().any(|e| matches!(
        e,
        AgentEvent::ExecutionFinished { execution } if execution.state == ExecutionState::Cancelled
    )));
}

/// A clock that tests move forward.
fn offset_clock() -> (Clock, Arc<AtomicU64>) {
    let offset = Arc::new(AtomicU64::new(0));
    let clock: Clock = {
        let offset = offset.clone();
        Arc::new(move || SystemTime::now() + Duration::from_secs(offset.load(Ordering::SeqCst)))
    };
    (clock, offset)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn triggered_runs_respect_manual_runs_and_the_concurrency_limit() {
    let mut profile = work_profile();
    let mut watched = waiting_workflow("Watched", 1_000);
    watched.triggers = vec![
        WorkflowTrigger::lifecycle(LifecycleEvent::DeviceConnected),
        WorkflowTrigger::schedule(Schedule::Interval { minutes: 1 }),
    ];
    let busy: Vec<Workflow> = (1..=4)
        .map(|n| waiting_workflow(&format!("Busy {n}"), 1_500))
        .collect();
    profile.workflows.push(watched.clone());
    profile.workflows.extend(busy.iter().cloned());
    let dir = TempDir::new("deckpilot-e2e-triggers");
    let (paths, key) = support::data_dir(&dir, &profile, &[]);
    let (clock, offset) = offset_clock();
    let agent = TestAgent::start_unlocked(&paths, &key, &MockBackend::new(), Some(clock)).await;

    // A manual run is going: a device connection does not start another.
    let manual = agent.start_workflow(watched.id).await.unwrap();
    agent.mock.plug(&models::stream_deck_mk2(), "SIM-0001");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(agent.runs(watched.id).await.len(), 1);
    agent.wait_finished(manual.id).await;

    // A triggered run is going: a manual start is refused.
    agent.mock.plug(&models::stream_deck_mk2(), "SIM-0002");
    let runs = agent
        .wait_for_runs(watched.id, |runs| runs.len() == 2)
        .await;
    let triggered = &runs[0];
    assert_eq!(triggered.trigger, ExecutionTrigger::Lifecycle);
    assert_eq!(triggered.trigger_detail.as_deref(), Some("deviceConnected"));
    let refused = agent.start_workflow(watched.id).await.unwrap_err();
    assert!(refused.to_string().contains("already running"), "{refused}");
    agent.wait_finished(triggered.id).await;

    // Four workflows run: the schedule falls due and is skipped, not queued.
    let mut started = Vec::new();
    for workflow in &busy {
        started.push(agent.start_workflow(workflow.id).await.unwrap().id);
    }
    offset.fetch_add(61, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(agent.runs(watched.id).await.len(), 2);
    for id in started {
        assert_eq!(
            agent.wait_finished(id).await.summary.state,
            ExecutionState::Succeeded
        );
    }
    // Nothing is made up when they finish; the next due time starts it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(agent.runs(watched.id).await.len(), 2);
    offset.fetch_add(60, Ordering::SeqCst);
    let runs = agent
        .wait_for_runs(watched.id, |runs| runs.len() == 3)
        .await;
    assert_eq!(runs[0].trigger, ExecutionTrigger::Schedule);
    assert_eq!(runs[0].trigger_detail.as_deref(), Some("every 1 min"));
    assert_eq!(
        agent.wait_finished(runs[0].id).await.summary.state,
        ExecutionState::Succeeded
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_interrupted_by_a_restart_is_recorded_and_new_runs_work() {
    let mut profile = work_profile();
    let long = waiting_workflow("Long", 30_000);
    let quick = waiting_workflow("Quick", 10);
    profile.workflows.extend([long.clone(), quick.clone()]);
    let dir = TempDir::new("deckpilot-e2e-restart");
    let (paths, key) = support::data_dir(&dir, &profile, &[]);
    let mock = MockBackend::new();

    let agent = TestAgent::start_unlocked(&paths, &key, &mock, None).await;
    let run = agent.start_workflow(long.id).await.unwrap();
    agent.wait_for_node(run.id, "wait").await;
    agent.shutdown().await;

    let agent = TestAgent::start_unlocked(&paths, &key, &mock, None).await;
    let record = agent.execution(run.id).await;
    assert_eq!(record.summary.state, ExecutionState::Interrupted);
    assert!(record.summary.finished_at.is_some());
    assert_eq!(
        record
            .nodes
            .iter()
            .find(|n| n.node.as_str() == "wait")
            .map(|n| n.status),
        Some(NodeStatus::Cancelled)
    );
    // The interrupted workflow can run again, and others run.
    let again = agent.start_workflow(long.id).await.unwrap();
    agent.wait_for_node(again.id, "wait").await;
    agent
        .request(Request::CancelExecution {
            execution: again.id,
        })
        .await
        .unwrap();
    assert_eq!(
        agent.wait_finished(again.id).await.summary.state,
        ExecutionState::Cancelled
    );
    let other = agent.start_workflow(quick.id).await.unwrap();
    assert_eq!(
        agent.wait_finished(other.id).await.summary.state,
        ExecutionState::Succeeded
    );
    let runs = agent.runs(long.id).await;
    assert_eq!(runs.len(), 2);
}
