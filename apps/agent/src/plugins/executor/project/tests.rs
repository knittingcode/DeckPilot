//! Project operations and `profile.read`: grants, run origins (no switch
//! from a lifecycle event, a schedule or a state call), the agent's
//! refusals, scopes and the audit.

use std::sync::{Arc, Mutex};

use deckpilot_plugin_abi::CapabilityRequest;
use deckpilot_plugin_abi::capability::{RunCause, RunContext, RunTrigger};
use deckpilot_plugin_broker::{Broker, Decision};
use deckpilot_plugin_supervisor::CapabilityExecutor;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::super::{
    Executor, Opener, OwnVariables, PrivilegedAudit, PrivilegedEntry, PublishedVariables, Transport,
};
use super::*;
use crate::service::plugin_projects::ProjectReply;

const PLUGIN: &str = "deckpilot.projects";
/// No call reached the agent.
const NONE: [ProjectCall; 0] = [];

/// The agent's side, recorded: every call it received, and its answer.
#[derive(Debug)]
struct FakeAgent {
    calls: Mutex<Vec<(String, ProjectCall)>>,
    /// `false`: no project context provider is active.
    provider: bool,
}

fn entry(id: &str, key: &str) -> ProjectEntry {
    ProjectEntry {
        id: id.into(),
        key: key.into(),
        name: format!("{key} project"),
    }
}

impl ProjectHost for FakeAgent {
    fn call(&self, plugin: &str, call: ProjectCall) -> ProjectReply {
        self.calls
            .lock()
            .unwrap()
            .push((plugin.to_owned(), call.clone()));
        if !self.provider && call != ProjectCall::Profile {
            return Err(ProjectRefusal::new(
                "projects-off",
                "no project context is available: the Projects feature is off",
            ));
        }
        Ok(match call {
            // A workflow run's project: answered for it, a switch planned
            // from it.
            ProjectCall::InRun { project, call } => match *call {
                ProjectCall::Current => ProjectAnswer::Current(project.map(|id| entry(&id, "RUN"))),
                ProjectCall::Select { id } => ProjectAnswer::Switched {
                    from: project,
                    to: entry(&id, "CRM"),
                },
                ProjectCall::Cycle { .. } => ProjectAnswer::Switched {
                    from: project,
                    to: entry("c", "OPS"),
                },
                other => panic!("not a run's call: {other:?}"),
            },
            ProjectCall::Current => ProjectAnswer::Current(Some(entry("a", "SHOP"))),
            ProjectCall::List => ProjectAnswer::List(vec![entry("a", "SHOP"), entry("b", "CRM")]),
            ProjectCall::Select { id } if id == "b" => ProjectAnswer::Switched {
                from: Some("a".into()),
                to: entry("b", "CRM"),
            },
            ProjectCall::Select { .. } => {
                return Err(ProjectRefusal::new(
                    "not-found",
                    "the active profile has no such project",
                ));
            }
            ProjectCall::Cycle { .. } => ProjectAnswer::Switched {
                from: Some("a".into()),
                to: entry("b", "CRM"),
            },
            ProjectCall::Profile => ProjectAnswer::Profile {
                name: "Work".into(),
                project: self.provider.then(|| entry("a", "SHOP")),
            },
        })
    }
}

struct Fixture {
    broker: Broker,
    clock: Arc<deckpilot_plugin_broker::ManualClock>,
    agent: Arc<FakeAgent>,
    audit: Arc<Mutex<Vec<PrivilegedEntry>>>,
}

impl Fixture {
    fn new(grants: &str) -> Self {
        Self::with_provider(grants, true)
    }

    fn with_provider(grants: &str, provider: bool) -> Self {
        let clock = Arc::new(deckpilot_plugin_broker::ManualClock::default());
        let broker = Broker::new(
            Arc::new(deckpilot_plugin_broker::MemoryAudit::new(100)),
            clock.clone(),
            deckpilot_plugin_broker::RateLimits::default(),
        );
        broker
            .set_grants(
                &PluginId::new(PLUGIN).unwrap(),
                &deckpilot_plugin_broker::ApprovedPermissions::from_json(grants).unwrap(),
            )
            .unwrap();
        Self {
            broker,
            clock,
            agent: Arc::new(FakeAgent {
                calls: Mutex::new(Vec::new()),
                provider,
            }),
            audit: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// `projectContext` with `select` (high), and `profile` reading both
    /// scopes.
    fn switching() -> Self {
        Self::new(
            r#"[{"kind":"projectContext","select":true},
                {"kind":"profile","read":["profile","project"]}]"#,
        )
    }

    /// The executor of an invocation of an action that changes the run's
    /// project (`changes`), in a run whose project is `project`.
    fn invocation(
        &self,
        run: RunContext,
        changes: bool,
        project: Option<&str>,
    ) -> (Executor, ProjectSwitch) {
        let switch = ProjectSwitch::for_action(changes);
        let resources = super::super::resources::InvocationResources {
            project: project.map(|id| super::super::resources::ProjectResources {
                id: id.into(),
                roles: std::collections::BTreeMap::new(),
            }),
            switch: switch.clone(),
            ..Default::default()
        };
        let (executor, _) = self.executor(run).with_resources(
            PLUGIN,
            "0.1.0",
            std::collections::BTreeMap::new(),
            resources,
        );
        (executor, switch)
    }

    /// One call of `executor`, as [`Fixture::call`].
    fn call_in(&self, executor: &mut Executor, request: &Value) -> CallResult {
        let plugin = PluginId::new(PLUGIN).unwrap();
        let request: CapabilityRequest = serde_json::from_value(request.clone()).unwrap();
        let session = self.broker.session(plugin.clone());
        match session.authorize_in(&request, executor.run_context()) {
            Decision::Allow(allowed) => executor.execute(&plugin, &allowed),
            Decision::Deny(reason) => {
                CallResult::error(reason.call_error_code(), reason.to_string())
            }
        }
    }

    fn executor(&self, run: RunContext) -> Executor {
        let (events, _) = broadcast::channel(4);
        let sink = self.audit.clone();
        let audit: PrivilegedAudit = Arc::new(move |entry: &PrivilegedEntry| {
            sink.lock().unwrap().push(entry.clone());
        });
        Executor::new(
            self.broker.clone(),
            "Projects".into(),
            events,
            Transport::Network,
            OwnVariables {
                store: PublishedVariables::default(),
                namespace: "plugins.deckpilot.projects".into(),
                generation: 0,
                declared: Vec::new(),
            },
            Opener::System,
        )
        .with_audit(audit)
        .with_projects(Some(self.agent.clone() as Arc<dyn ProjectHost>))
        .with_run(run)
    }

    /// What the supervisor does with one call: the broker decides with the
    /// executor's run, the executor carries out what it allowed.
    fn call(&self, run: RunContext, request: &Value) -> CallResult {
        let mut executor = self.executor(run);
        let plugin = PluginId::new(PLUGIN).unwrap();
        let request: CapabilityRequest = serde_json::from_value(request.clone()).unwrap();
        let session = self.broker.session(plugin.clone());
        match session.authorize_in(&request, executor.run_context()) {
            Decision::Allow(allowed) => executor.execute(&plugin, &allowed),
            Decision::Deny(reason) => {
                CallResult::error(reason.call_error_code(), reason.to_string())
            }
        }
    }

    fn calls(&self) -> Vec<ProjectCall> {
        self.agent
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(plugin, call)| {
                assert_eq!(plugin, PLUGIN);
                call.clone()
            })
            .collect()
    }
}

fn key() -> RunContext {
    RunContext::direct(RunTrigger::Key)
}

fn select(id: &str) -> Value {
    json!({"op": "project.select", "args": {"id": id}})
}

fn cycle() -> Value {
    json!({"op": "project.cycle", "args": {"direction": "next"}})
}

fn ok(result: CallResult) -> Value {
    match result {
        CallResult::Ok(value) => value,
        CallResult::Error(e) => panic!("refused: {e:?}"),
    }
}

fn refused(result: &CallResult) -> (CallErrorCode, String) {
    match result {
        CallResult::Error(e) => (e.code, e.message.clone()),
        CallResult::Ok(value) => panic!("allowed: {value}"),
    }
}

#[test]
fn a_select_in_a_run_the_user_started_switches_and_is_audited() {
    let f = Fixture::switching();
    let switched = ok(f.call(key(), &select("b")));
    assert_eq!(switched["key"], "CRM");
    assert_eq!(f.calls(), vec![ProjectCall::Select { id: "b".into() }]);
    let audit = f.audit.lock().unwrap();
    let record = &audit[0].record;
    assert_eq!(
        (record.op, record.grant, record.outcome),
        ("project.select", "projectContext:select", None)
    );
    assert_eq!(
        record.details,
        vec![("from", "a".to_owned()), ("to", "b".to_owned())]
    );
    assert_eq!(audit[0].origin, "key");
    drop(audit);

    // 2 switches per second without a burst: a second one within 500 ms
    // is throttled, one 500 ms later is not.
    let (code, _) = refused(&f.call(key(), &select("b")));
    assert_eq!(code, CallErrorCode::Throttled);
    f.clock.advance(std::time::Duration::from_millis(500));
    ok(f.call(key(), &select("b")));
    f.clock.advance(std::time::Duration::from_millis(500));
    // A workflow the user started may switch too, its run's project
    // (below); cycling wraps by default.
    let user = RunContext::workflow(RunCause::User, None);
    let (mut executor, switch) = f.invocation(user, true, Some("a"));
    ok(f.call_in(&mut executor, &cycle()));
    assert_eq!(f.calls().len(), 3);
    assert_eq!(
        f.calls()[2],
        ProjectCall::InRun {
            project: Some("a".into()),
            call: Box::new(ProjectCall::Cycle {
                direction: CoreDirection::Next,
                wrap: true
            }),
        }
    );
    assert_eq!(switch.target().as_deref(), Some("c"));
}

/// In a workflow the user started, a switch is the run's: the agent only
/// plans it from the run's project, the invocation keeps the target for
/// the action's result (the engine applies it to the nodes after it), and
/// `current` and `profile.read` then answer the run's new project. An
/// action whose node does not declare `changesContext` cannot switch.
#[test]
fn a_switch_in_a_workflow_run_selects_the_runs_project() {
    let f = Fixture::switching();
    let user = RunContext::workflow(RunCause::User, None);
    let (mut executor, switch) = f.invocation(user.clone(), true, Some("a"));
    let switched = ok(f.call_in(&mut executor, &select("b")));
    assert_eq!(switched["id"], "b");
    assert_eq!(switch.target().as_deref(), Some("b"));
    let current = ok(f.call_in(&mut executor, &json!({"op": "project.current", "args": {}})));
    assert_eq!(current["id"], "b");
    assert_eq!(
        f.calls(),
        vec![
            ProjectCall::InRun {
                project: Some("a".into()),
                call: Box::new(ProjectCall::Select { id: "b".into() }),
            },
            ProjectCall::InRun {
                project: Some("b".into()),
                call: Box::new(ProjectCall::Current),
            },
        ]
    );
    let audit = f.audit.lock().unwrap();
    assert_eq!(
        audit[0].record.details,
        vec![
            ("from", "a".to_owned()),
            ("to", "b".to_owned()),
            ("scope", "run".to_owned())
        ]
    );
    assert_eq!(audit[0].origin, "workflow/user");
    drop(audit);

    // Not an action that changes the run's project: refused, the agent
    // never asked, nothing selected.
    f.clock.advance(std::time::Duration::from_secs(1));
    let (mut executor, switch) = f.invocation(user, false, Some("a"));
    let (code, message) = refused(&f.call_in(&mut executor, &select("b")));
    assert_eq!(code, CallErrorCode::NotPermitted);
    assert!(message.contains("changesContext"), "{message}");
    assert_eq!(switch.target(), None);
    assert_eq!(f.calls().len(), 2);

    // A lifecycle-triggered workflow cannot switch, even in an action
    // that may.
    let (mut executor, switch) =
        f.invocation(RunContext::workflow(RunCause::Event, None), true, Some("a"));
    let (code, _) = refused(&f.call_in(&mut executor, &select("b")));
    assert_eq!(code, CallErrorCode::NotPermitted);
    assert_eq!(switch.target(), None);
    assert_eq!(f.calls().len(), 2);
}

#[test]
fn no_switch_from_a_lifecycle_event_a_schedule_or_a_state_call() {
    let f = Fixture::switching();
    for run in [
        // A workflow the `projectChanged` lifecycle trigger started: the
        // loop guard.
        RunContext::workflow(RunCause::Event, None),
        RunContext::workflow(RunCause::Schedule, None),
        RunContext::workflow(RunCause::Startup, None),
        RunContext::workflow(RunCause::WorkSession, None),
        // A key's state call: a run nobody started.
        RunContext::unknown(),
    ] {
        for request in [select("b"), cycle()] {
            let (code, _) = refused(&f.call(run.clone(), &request));
            assert_eq!(code, CallErrorCode::NotPermitted, "{run:?}");
        }
    }
    assert_eq!(f.calls(), NONE, "the agent was never asked");

    // The executor checks the run itself too, whatever it was handed.
    let executor = f.executor(RunContext::workflow(RunCause::Event, None));
    let plugin = PluginId::new(PLUGIN).unwrap();
    let (code, message) =
        refused(&executor.project(&plugin, &Allowed::ProjectSelect { id: "b".into() }));
    assert_eq!(code, CallErrorCode::NotPermitted);
    assert!(message.starts_with("not-permitted: "), "{message}");
    assert_eq!(f.calls(), NONE);
    let audit = f.audit.lock().unwrap();
    assert_eq!(audit.last().unwrap().record.outcome, Some("not-permitted"));
    assert_eq!(audit.last().unwrap().origin, "workflow/event");
}

#[test]
fn switching_needs_the_select_grant() {
    let reading = Fixture::new(r#"[{"kind":"projectContext"}]"#);
    for request in [select("b"), cycle()] {
        let (code, _) = refused(&reading.call(key(), &request));
        assert_eq!(code, CallErrorCode::NotPermitted);
    }
    let none = Fixture::new(r#"[{"kind":"profile","read":["profile"]}]"#);
    for op in ["project.current", "project.list", "project.select"] {
        let request = if op == "project.select" {
            select("b")
        } else {
            json!({"op": op, "args": {}})
        };
        let (code, _) = refused(&none.call(key(), &request));
        assert_eq!(code, CallErrorCode::NotPermitted, "{op}");
    }
    assert_eq!(reading.calls(), NONE);
    assert_eq!(none.calls(), NONE);
}

#[test]
fn reading_works_with_the_medium_grant_in_any_run() {
    let f = Fixture::new(r#"[{"kind":"projectContext"}]"#);
    let current = ok(f.call(key(), &json!({"op": "project.current", "args": {}})));
    assert_eq!(
        current,
        json!({"id": "a", "key": "SHOP", "name": "SHOP project"})
    );
    // In a workflow (one an event started, too) `current` is the run's
    // project.
    let run = RunContext::workflow(RunCause::Event, None);
    let (mut executor, _) = f.invocation(run.clone(), false, Some("r"));
    let in_run = ok(f.call_in(&mut executor, &json!({"op": "project.current", "args": {}})));
    assert_eq!(in_run["id"], "r");
    let list = ok(f.call(run, &json!({"op": "project.list", "args": {}})));
    assert_eq!(list["projects"].as_array().unwrap().len(), 2);
    // Ids, keys and names only.
    assert_eq!(
        list["projects"][1],
        json!({"id": "b", "key": "CRM", "name": "CRM project"})
    );
    assert_eq!(
        f.calls(),
        vec![
            ProjectCall::Current,
            ProjectCall::InRun {
                project: Some("r".into()),
                call: Box::new(ProjectCall::Current),
            },
            ProjectCall::List
        ]
    );
    let audit = f.audit.lock().unwrap();
    assert_eq!(audit[0].record.grant, "projectContext");
    assert_eq!(audit.len(), 3);
}

#[test]
fn the_agents_refusals_carry_their_reason_code() {
    let off = Fixture::with_provider(r#"[{"kind":"projectContext","select":true}]"#, false);
    for request in [
        json!({"op": "project.current", "args": {}}),
        json!({"op": "project.list", "args": {}}),
        select("b"),
    ] {
        let (code, message) = refused(&off.call(key(), &request));
        assert_eq!(code, CallErrorCode::Unavailable);
        assert!(message.starts_with("projects-off: "), "{message}");
    }
    let f = Fixture::switching();
    let (code, message) = refused(&f.call(key(), &select("other")));
    assert_eq!(code, CallErrorCode::Failed);
    assert!(message.starts_with("not-found: "), "{message}");

    // Without the agent's project context nothing is answered.
    let plugin = PluginId::new(PLUGIN).unwrap();
    let executor = f.executor(key()).with_projects(None);
    let (code, _) = refused(&executor.project(&plugin, &Allowed::ProjectCurrent));
    assert_eq!(code, CallErrorCode::Unavailable);
}

#[test]
fn the_profile_read_answers_only_the_granted_scopes() {
    let read = json!({"op": "profile.read", "args": {}});
    let both = Fixture::switching();
    assert_eq!(
        ok(both.call(key(), &read)),
        json!({"profile": "Work", "project": {"id": "a", "key": "SHOP", "name": "SHOP project"}})
    );
    let profile = Fixture::new(r#"[{"kind":"profile","read":["profile"]}]"#);
    assert_eq!(ok(profile.call(key(), &read)), json!({"profile": "Work"}));
    let project = Fixture::new(r#"[{"kind":"profile","read":["project"]}]"#);
    assert_eq!(ok(project.call(key(), &read))["project"]["key"], "SHOP");
    assert!(ok(project.call(key(), &read)).get("profile").is_none());
    // No active project (or Projects off): no `project`, not an error.
    let off = Fixture::with_provider(r#"[{"kind":"profile","read":["project"]}]"#, false);
    assert_eq!(ok(off.call(key(), &read)), json!({}));
    // Not granted: refused before the agent is asked.
    let none = Fixture::new(r#"[{"kind":"projectContext"}]"#);
    let (code, _) = refused(&none.call(key(), &read));
    assert_eq!(code, CallErrorCode::NotPermitted);
    assert_eq!(none.calls(), NONE);
}

#[test]
fn a_host_published_variable_is_never_the_plugins_to_publish() {
    let f = Fixture::switching();
    let variable: deckpilot_plugin_manifest::Variable = serde_json::from_value(json!({
        "id": "current.key", "name": "Project key", "valueType": {"type": "string"},
        "scope": "project", "source": "host:project.key"
    }))
    .unwrap();
    let mut executor = f.executor(key());
    executor.variables.declared = vec![variable];
    let plugin = PluginId::new(PLUGIN).unwrap();
    for allowed in [
        Allowed::VariablePublish {
            id: "current.key".into(),
            value: json!("FORGED"),
        },
        Allowed::VariableClear {
            id: Some("current.key".into()),
        },
    ] {
        let (code, message) = refused(&executor.execute(&plugin, &allowed));
        assert_eq!(code, CallErrorCode::NotPermitted);
        assert!(message.contains("published by DeckPilot"), "{message}");
    }
}
