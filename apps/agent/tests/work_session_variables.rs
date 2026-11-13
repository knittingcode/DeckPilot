//! The work session variables (`system.work.session.…`,
//! docs/gestures-and-work-session.md §8.1) end to end: listed by
//! `listVariables`, previewed with live values over IPC before, during and
//! after a session, announced when they change, and resolved in workflow
//! runs at the moment the run reads them.

mod support;

use std::time::Duration;

use deckpilot_agent::{WorkSessionOperation as Op, WorkSessionOutcome};
use deckpilot_core::gesture::Gesture;
use deckpilot_core::variables::{
    CatalogEntry, CatalogFilter, PreviewItem, ReferenceAvailability, ResolutionContext,
};
use deckpilot_core::work_session::{WorkSessionSnapshot, WorkSessionState as S};
use deckpilot_core::{
    ActionParameters, ParameterKind, ParameterValue, Profile, Project, ProjectKey,
};
use deckpilot_device::mock::MockBackend;
use deckpilot_ipc::{AgentClient, AgentEvent, Request, Response};
use deckpilot_modules::general;
use serde_json::{Value, json};
use support::{Jump, TIMEOUT, TempDir, TestAgent, data_dir};

const PATHS: [&str; 6] = [
    "system.work.session.id",
    "system.work.session.state",
    "system.work.session.startedAt",
    "system.work.session.elapsedSeconds",
    "system.work.session.projectId",
    "system.work.session.canFinish",
];

struct Setup {
    agent: TestAgent,
    profile: Profile,
    jump: Jump,
    _dir: TempDir,
}

/// A profile with a project and a workflow `Report` whose action node logs
/// the session's state and elapsed seconds.
async fn setup() -> Setup {
    let dir = TempDir::new("deckpilot-work-session-variables");
    let mut profile = Profile::new("Work");
    profile
        .projects
        .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
    let log = support::registry()
        .create_instance(
            "Log",
            general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text("unused".into())),
        )
        .unwrap();
    let log_id = log.id;
    profile.actions.push(log);
    profile.workflows.push(support::graph_workflow(
        "Report",
        support::graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "report", "type": "action", "action": log_id, "mappings": {
                    "message": {"type": "template", "template":
                        "{{ system.work.session.state }} for {{ system.work.session.elapsedSeconds }}s"},
                }},
            ]),
            &json!([{"from": "start", "to": "report"}]),
        ),
    ));
    let (paths, key) = data_dir(&dir, &profile, &[]);
    let mock = MockBackend::new();
    let jump = Jump::default();
    let agent = TestAgent::start_unlocked(&paths, &key, &mock, Some(jump.clock())).await;
    agent
        .request(Request::SelectProject {
            project: profile.projects[0].id,
        })
        .await
        .unwrap();
    Setup {
        agent,
        profile,
        jump,
        _dir: dir,
    }
}

impl Setup {
    async fn preview(&self, items: &[(&str, Option<ParameterKind>)]) -> Vec<Value> {
        let items = items
            .iter()
            .map(|(reference, expected)| PreviewItem {
                reference: (*reference).to_owned(),
                expected: expected.clone(),
            })
            .collect();
        let response = self
            .agent
            .request(Request::PreviewReferences {
                context: Box::new(ResolutionContext::preview(self.profile.id)),
                items,
            })
            .await
            .unwrap();
        let Response::ReferencePreviews { previews } = response else {
            panic!("previews expected, got {response:?}");
        };
        let json = serde_json::to_value(previews).unwrap();
        json.as_array().unwrap().clone()
    }

    /// The preview of every path, by path.
    async fn values(&self) -> Vec<Value> {
        let items: Vec<_> = PATHS.iter().map(|p| (*p, None)).collect();
        self.preview(&items)
            .await
            .into_iter()
            .map(|p| p["status"].clone())
            .collect()
    }

    async fn list(&self) -> Vec<CatalogEntry> {
        let response = self
            .agent
            .request(Request::ListVariables {
                context: Box::new(ResolutionContext::preview(self.profile.id)),
                filter: Box::new(CatalogFilter {
                    query: "work session".into(),
                    ..CatalogFilter::default()
                }),
            })
            .await
            .unwrap();
        let Response::Variables { entries } = response else {
            panic!("entries expected, got {response:?}");
        };
        entries
    }

    async fn op(&self, operation: Op, gesture: Gesture) -> WorkSessionOutcome {
        self.agent
            .handle
            .work_session_operation(operation, gesture, None)
            .await
            .expect("the agent answers")
    }

    async fn wait_state(&self, state: S) -> WorkSessionSnapshot {
        let mut updates = self.agent.handle.work_session();
        tokio::time::timeout(TIMEOUT, async {
            loop {
                {
                    let current = updates.borrow_and_update();
                    if current.state == state {
                        return current.clone();
                    }
                }
                updates.changed().await.expect("the agent runs");
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{} not reached", state.name()))
    }

    /// Runs `Report`: what its node logged, or why it failed.
    async fn report(&self) -> Result<String, String> {
        let execution = self
            .agent
            .start_workflow(self.profile.workflows[0].id)
            .await
            .unwrap()
            .id;
        let record = self.agent.wait_finished(execution).await;
        let node = record
            .nodes
            .iter()
            .find(|n| n.node.as_str() == "report")
            .unwrap_or_else(|| panic!("no report node in {record:#?}"));
        match &node.error {
            None => Ok(node.message.clone()),
            Some(error) => Err(format!("{}: {}", error.code, error.message)),
        }
    }
}

/// The next `sharedVariablesChanged` naming work session paths.
async fn changed_paths(events: &mut AgentClient) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("an announcement in time")
            .unwrap();
        if let AgentEvent::SharedVariablesChanged { keys } = event
            && keys.iter().any(|k| k.starts_with("system.work.session."))
        {
            return keys;
        }
    }
}

fn status(value: &Value) -> &str {
    value["status"].as_str().unwrap()
}

/// A minute later, without any transition, previews and runs see the
/// time: it is computed when read.
async fn elapsed_is_computed_when_read(s: &Setup) {
    s.jump.by(61_000);
    let elapsed = &s.values().await[3];
    assert!(
        elapsed["value"]["value"].as_i64().unwrap() >= 61,
        "{elapsed}"
    );
    let logged = s.report().await.unwrap();
    let seconds: i64 = logged
        .strip_prefix("working for ")
        .and_then(|rest| rest.strip_suffix('s'))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{logged}"));
    assert!(seconds >= 61, "{logged}");
}

/// Types are checked against the parameter that receives them.
async fn types_are_checked(s: &Setup) {
    let typed = s
        .preview(&[
            (
                "system.work.session.elapsedSeconds",
                Some(ParameterKind::Integer {
                    min: None,
                    max: None,
                }),
            ),
            (
                "system.work.session.elapsedSeconds",
                Some(ParameterKind::Project),
            ),
            (
                "system.work.session.projectId",
                Some(ParameterKind::Project),
            ),
            ("system.work.session.state", Some(ParameterKind::Boolean)),
        ])
        .await;
    assert_eq!(status(&typed[0]["status"]), "resolved");
    assert_eq!(status(&typed[1]["status"]), "typeMismatch");
    assert_eq!(status(&typed[2]["status"]), "resolved");
    assert_eq!(status(&typed[3]["status"]), "typeMismatch");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_session_variables_follow_the_session_live() {
    let s = setup().await;
    let shop = s.profile.projects[0].id;

    // Idle: listed (a workflow may be written for later), no values.
    let entries = s.list().await;
    let listed: Vec<String> = entries.iter().map(|e| e.reference.to_string()).collect();
    assert_eq!(listed, PATHS);
    assert!(entries.iter().all(|e| e.dynamic
        && !e.sensitive
        && e.availability == ReferenceAvailability::RuntimeOnly));
    for value in s.values().await {
        assert_eq!(status(&value), "missing", "{value}");
        assert_eq!(value["reason"], "noValue", "{value}");
    }
    // A run without a session stops at the missing reference.
    let error = s.report().await.unwrap_err();
    assert!(
        error.starts_with("reference-missing") && error.contains("system.work.session.state"),
        "{error}"
    );

    // Start: every variable has a value, announced to subscribers.
    let mut events = s.agent.events().await;
    assert_eq!(
        s.op(Op::Start, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    let working = s.wait_state(S::Working).await;
    let mut announced = changed_paths(&mut events).await;
    announced.sort();
    let mut all = PATHS.map(str::to_owned).to_vec();
    all.sort();
    assert_eq!(announced, all);
    assert!(
        s.list()
            .await
            .iter()
            .all(|e| e.availability == ReferenceAvailability::Available)
    );
    let values = s.values().await;
    for value in &values[..5] {
        assert_eq!(status(value), "resolved", "{value}");
    }
    assert_eq!(
        values[0]["value"],
        json!({"type": "string", "value": working.session.unwrap().to_string()})
    );
    assert_eq!(values[1]["value"]["value"], "working");
    let started = values[2]["value"]["value"].as_str().unwrap();
    assert!(
        started.ends_with('Z') && started.len() == "2026-11-12T08:00:00Z".len(),
        "{started}"
    );
    assert_eq!(values[3]["value"]["type"], "integer");
    assert_eq!(
        values[4]["value"],
        json!({"type": "projectId", "value": shop.to_string()})
    );
    // Not verified yet.
    assert_eq!(status(&values[5]), "missing");

    elapsed_is_computed_when_read(&s).await;
    types_are_checked(&s).await;

    // Verify (no verification workflow: a long press approves at once).
    assert_eq!(
        s.op(Op::Verify, Gesture::LongPress).await,
        WorkSessionOutcome::Applied
    );
    let ready = s.wait_state(S::ReadyToFinish).await;
    let mut keys = changed_paths(&mut events).await;
    while !keys.iter().any(|k| k == "system.work.session.canFinish") {
        keys = changed_paths(&mut events).await;
    }
    let values = s.values().await;
    assert_eq!(values[1]["value"]["value"], "readyToFinish");
    assert_eq!(
        values[5]["value"],
        json!({"type": "boolean", "value": true})
    );
    // The verification's details are not a variable.
    let details = ready
        .verification
        .unwrap()
        .outcome
        .unwrap()
        .details
        .unwrap();
    assert!(!Value::Array(values).to_string().contains(&details));

    // Finish: back to no values, announced.
    tokio::time::sleep(
        deckpilot_agent::service::work_session::ARMING_DELAY + Duration::from_millis(100),
    )
    .await;
    assert_eq!(
        s.op(Op::Finish, Gesture::Click).await,
        WorkSessionOutcome::Applied
    );
    s.wait_state(S::Idle).await;
    let mut keys = changed_paths(&mut events).await;
    while !keys.iter().any(|k| k == "system.work.session.id") {
        keys = changed_paths(&mut events).await;
    }
    for value in s.values().await {
        assert_eq!(value["reason"], "noValue", "{value}");
    }
    s.agent.shutdown().await;
}
