use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::testing::{Select, select_type, with_projects};
use crate::{
    ActionHandler, ActionInstance, ActionRegistry, ActionResult, ActionType, ActionTypeId,
    ActiveProjectContext, Engine, ExecutionContext, MachineConfig, OnFailure, Os, ParameterKind,
    ParameterSpec, ParameterValue, Profile, StepCondition, Workflow, WorkflowStep,
};

/// `test.probe`: records that it ran, optionally sleeps, fails the first
/// `fail_times` calls, and outputs `value` = its `message`.
struct Probe {
    definition: ActionType,
    log: Arc<Mutex<Vec<String>>>,
    calls: Arc<AtomicU32>,
}

fn probe_type() -> ActionTypeId {
    ActionTypeId::new("test.probe").unwrap()
}

fn parameter(key: &str, kind: ParameterKind) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: key.into(),
        description: None,
        kind,
        required: false,
        default: None,
    }
}

impl ActionHandler for Probe {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &crate::ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let message = parameters.text("message").unwrap_or_default().to_owned();
        self.log.lock().unwrap().push(message.clone());
        if let Some(ms) = parameters.integer("sleep_ms") {
            std::thread::sleep(Duration::from_millis(u64::try_from(ms).unwrap()));
        }
        // Cooperative: waits through the context and stops when cancelled.
        if let Some(ms) = parameters.integer("wait_ms") {
            context.report_progress(Some(0), "waiting");
            if !context.wait(Duration::from_millis(u64::try_from(ms).unwrap())) {
                return ActionResult::failure(format!("{message} stopped"));
            }
        }
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let fail_times = parameters.integer("fail_times").unwrap_or(0);
        if message == "fail" || i64::from(call) < fail_times {
            return ActionResult::failure(format!("{message} failed"));
        }
        ActionResult::success(format!("ran {message}")).with_data("value", message)
    }
}

struct Fixture {
    engine: Engine,
    profile: Profile,
    log: Arc<Mutex<Vec<String>>>,
    calls: Arc<AtomicU32>,
}

impl Fixture {
    fn new() -> Self {
        let log = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicU32::new(0));
        let mut registry = ActionRegistry::default();
        registry
            .register(Box::new(Probe {
                definition: ActionType {
                    id: probe_type(),
                    name: "Probe".into(),
                    description: String::new(),
                    category: "Test".into(),
                    parameters: vec![
                        parameter(
                            "message",
                            ParameterKind::Text {
                                multiline: false,
                                max_length: None,
                            },
                        ),
                        parameter(
                            "sleep_ms",
                            ParameterKind::Integer {
                                min: None,
                                max: None,
                            },
                        ),
                        parameter(
                            "fail_times",
                            ParameterKind::Integer {
                                min: None,
                                max: None,
                            },
                        ),
                        parameter(
                            "wait_ms",
                            ParameterKind::Integer {
                                min: None,
                                max: None,
                            },
                        ),
                    ],
                    requires_project: false,
                    node: NodeSpec {
                        outputs: vec![OutputSpec {
                            key: "value".into(),
                            label: "Value".into(),
                            kind: ValueKind::Text,
                            always: true,
                            sensitive: false,
                        }],
                        retry_safe: true,
                        idempotent: true,
                        ..NodeSpec::default()
                    },
                },
                log: log.clone(),
                calls: calls.clone(),
            }))
            .unwrap();
        registry.register(Box::new(Select::new())).unwrap();
        Self {
            engine: Engine::new(registry),
            profile: with_projects("Work", &["SHOP", "CMS"]),
            log,
            calls,
        }
    }

    /// Adds a probe action instance and returns its id.
    fn probe(
        &mut self,
        message: &str,
        extra: &[(&str, ParameterValue)],
    ) -> crate::ActionInstanceId {
        let mut parameters =
            crate::ActionParameters::new().with("message", ParameterValue::Text(message.into()));
        for (key, value) in extra {
            parameters = parameters.with(*key, value.clone());
        }
        self.add(probe_type(), message, parameters)
    }

    fn add(
        &mut self,
        action_type: ActionTypeId,
        name: &str,
        parameters: crate::ActionParameters,
    ) -> crate::ActionInstanceId {
        let instance = ActionInstance {
            id: crate::ActionInstanceId::new(),
            name: name.into(),
            action_type,
            parameters,
            description: String::new(),
            icon: None,
            label: None,
        };
        let id = instance.id;
        self.profile.actions.push(instance);
        id
    }

    fn workflow(&mut self, graph: WorkflowGraph) -> WorkflowId {
        let mut workflow = Workflow::new("Graph");
        workflow.graph = Some(graph);
        let id = workflow.id;
        self.profile.workflows.push(workflow);
        id
    }

    fn issues(&self, workflow: WorkflowId) -> Vec<GraphIssue> {
        let graph = self
            .profile
            .workflow(workflow)
            .unwrap()
            .graph
            .as_ref()
            .unwrap();
        graph.validate(workflow, &self.profile, self.engine.registry())
    }

    fn run_with(
        &self,
        workflow: WorkflowId,
        cancel: &CancellationToken,
    ) -> (GraphReport, Vec<ExecutionEvent>, ActiveProjectContext) {
        let events = Mutex::new(Vec::new());
        let mut context = ActiveProjectContext::new(&self.profile);
        let report = self
            .engine
            .run_graph(
                &self.profile,
                &MachineConfig::new(Os::current()),
                &mut context,
                workflow,
                cancel,
                &|event| events.lock().unwrap().push(event),
            )
            .unwrap();
        (report, events.into_inner().unwrap(), context)
    }

    fn run(&self, workflow: WorkflowId) -> GraphReport {
        self.run_with(workflow, &CancellationToken::new()).0
    }

    fn ran(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

fn n(id: &str) -> NodeId {
    NodeId::new(id).unwrap()
}

fn node(id: &str, kind: NodeKind) -> Node {
    Node {
        id: n(id),
        kind,
        label: String::new(),
        position: Position::default(),
    }
}

fn action(id: &str, action: crate::ActionInstanceId) -> Node {
    node(
        id,
        NodeKind::Action {
            action,
            overrides: crate::ActionParameters::new(),
            mappings: BTreeMap::new(),
            policy: ErrorPolicy::default(),
        },
    )
}

fn with_policy(mut node: Node, policy: ErrorPolicy) -> Node {
    if let NodeKind::Action { policy: p, .. } = &mut node.kind {
        *p = policy;
    }
    node
}

fn edge(from: &str, port: Port, to: &str) -> Edge {
    Edge {
        from: n(from),
        port,
        to: n(to),
    }
}

fn graph(nodes: Vec<Node>, edges: Vec<Edge>) -> WorkflowGraph {
    WorkflowGraph {
        version: GRAPH_VERSION,
        nodes,
        edges,
    }
}

fn status(report: &GraphReport, id: &str) -> NodeStatus {
    report.node(&n(id)).unwrap().status
}

#[test]
fn runs_in_order_and_passes_outputs_to_later_nodes() {
    let mut f = Fixture::new();
    let a = f.probe("alpha", &[]);
    let b = f.probe("unused", &[]);
    let mut second = action("second", b);
    if let NodeKind::Action { mappings, .. } = &mut second.kind {
        mappings.insert(
            "message".into(),
            Mapping::Template {
                template: "got {{ steps.first.output.value }}".into(),
            },
        );
    }
    let workflow = f.workflow(graph(
        vec![node("start", NodeKind::Trigger), action("first", a), second],
        vec![
            edge("start", Port::Out, "first"),
            edge("first", Port::Out, "second"),
        ],
    ));
    assert!(f.issues(workflow).is_empty(), "{:?}", f.issues(workflow));
    let (report, events, _) = f.run_with(workflow, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(f.ran(), ["alpha", "got alpha"]);
    assert_eq!(
        report.node(&n("second")).unwrap().output["value"],
        "got alpha"
    );
    assert_eq!(
        report
            .nodes
            .iter()
            .map(|r| r.node.as_str())
            .collect::<Vec<_>>(),
        ["start", "first", "second"]
    );
    // Every node starts and finishes, in order.
    let finished: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ExecutionEvent::NodeFinished { report } => Some(report.node.as_str().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(finished, ["start", "first", "second"]);
}

#[test]
fn conditions_choose_one_branch_and_skip_the_other() {
    let mut f = Fixture::new();
    let source = f.probe("yes", &[]);
    let then = f.probe("then", &[]);
    let otherwise = f.probe("otherwise", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            action("source", source),
            node(
                "check",
                NodeKind::Condition {
                    predicate: Predicate::Equals {
                        reference: Reference::parse("steps.source.output.value").unwrap(),
                        value: "yes".into(),
                    },
                },
            ),
            action("then", then),
            action("otherwise", otherwise),
            node("after", NodeKind::Join),
        ],
        vec![
            edge("start", Port::Out, "source"),
            edge("source", Port::Out, "check"),
            edge("check", Port::True, "then"),
            edge("check", Port::False, "otherwise"),
            edge("then", Port::Out, "after"),
            edge("otherwise", Port::Out, "after"),
        ],
    ));
    assert!(WorkflowGraph::is_runnable(&f.issues(workflow)));
    let report = f.run(workflow);
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(f.ran(), ["yes", "then"]);
    assert_eq!(status(&report, "otherwise"), NodeStatus::Skipped);
    // The join runs because one branch was taken.
    assert_eq!(status(&report, "after"), NodeStatus::Succeeded);
}

#[test]
fn parallel_branches_run_concurrently_and_join_once() {
    let mut f = Fixture::new();
    let sleep = ("sleep_ms", ParameterValue::Integer(300));
    let left = f.probe("left", std::slice::from_ref(&sleep));
    let right = f.probe("right", &[sleep]);
    let last = f.probe("last", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node("fork", NodeKind::Parallel),
            action("left", left),
            action("right", right),
            node("join", NodeKind::Join),
            action("last", last),
        ],
        vec![
            edge("start", Port::Out, "fork"),
            edge("fork", Port::Out, "left"),
            edge("fork", Port::Out, "right"),
            edge("left", Port::Out, "join"),
            edge("right", Port::Out, "join"),
            edge("join", Port::Out, "last"),
        ],
    ));
    assert!(f.issues(workflow).is_empty(), "{:?}", f.issues(workflow));
    let started = Instant::now();
    let report = f.run(workflow);
    let elapsed = started.elapsed();
    assert_eq!(report.status, RunStatus::Succeeded);
    // Sequentially this takes 600 ms; concurrently a little over 300 ms.
    assert!(
        elapsed < Duration::from_millis(550),
        "branches ran one after the other: {elapsed:?}"
    );
    assert_eq!(f.ran().last().map(String::as_str), Some("last"));
    assert_eq!(f.ran().iter().filter(|m| *m == "last").count(), 1);
    // Results are applied in a fixed order, whatever finished first.
    let order: Vec<_> = report.nodes.iter().map(|r| r.node.as_str()).collect();
    assert_eq!(order, ["start", "fork", "left", "right", "join", "last"]);
}

fn select_project(f: &mut Fixture, name: &str, key: &str) -> crate::ActionInstanceId {
    let project = f
        .profile
        .projects
        .iter()
        .find(|p| p.key.as_str() == key)
        .unwrap()
        .id;
    f.add(
        select_type(),
        name,
        crate::ActionParameters::new().with("project", crate::ParameterValue::Project(project)),
    )
}

fn is_project(id: &str, key: &str) -> Node {
    node(
        id,
        NodeKind::Condition {
            predicate: Predicate::ActiveProject { key: key.into() },
        },
    )
}

fn project_key(f: &Fixture, context: &ActiveProjectContext) -> String {
    context
        .project(&f.profile)
        .unwrap()
        .map(|p| p.key.to_string())
        .unwrap_or_default()
}

#[test]
fn parallel_branches_keep_their_own_project_context() {
    let mut f = Fixture::new();
    let select = select_project(&mut f, "Select CMS", "CMS");
    let saw_shop = f.probe("other branch saw SHOP", &[]);
    let after = f.probe("after the join: CMS", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node("fork", NodeKind::Parallel),
            action("select", select),
            is_project("check", "SHOP"),
            action("saw_shop", saw_shop),
            node("join", NodeKind::Join),
            is_project("check_after", "CMS"),
            action("after", after),
        ],
        vec![
            edge("start", Port::Out, "fork"),
            edge("fork", Port::Out, "select"),
            edge("fork", Port::Out, "check"),
            edge("check", Port::True, "saw_shop"),
            edge("select", Port::Out, "join"),
            edge("saw_shop", Port::Out, "join"),
            edge("join", Port::Out, "check_after"),
            edge("check_after", Port::True, "after"),
        ],
    ));
    // Project changes in parallel branches are allowed now.
    assert!(WorkflowGraph::is_runnable(&f.issues(workflow)));
    let (report, _, context) = f.run_with(workflow, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Succeeded, "{report:?}");
    // The other branch kept the initial project; the join took the change.
    assert_eq!(f.ran(), ["other branch saw SHOP", "after the join: CMS"]);
    assert_eq!(project_key(&f, &context), "CMS");
    assert_eq!(report.context_conflict, None);
}

#[test]
fn a_join_fails_when_branches_select_different_projects() {
    let mut f = Fixture::new();
    let to_cms = select_project(&mut f, "Select CMS", "CMS");
    let also_cms = select_project(&mut f, "Select CMS too", "CMS");
    let back = select_project(&mut f, "Back to SHOP", "SHOP");
    let after = f.probe("after", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node("fork", NodeKind::Parallel),
            action("a", to_cms),
            action("b", also_cms),
            action("b_back", back),
            node("join", NodeKind::Join),
            action("after", after),
        ],
        vec![
            edge("start", Port::Out, "fork"),
            edge("fork", Port::Out, "a"),
            edge("fork", Port::Out, "b"),
            edge("b", Port::Out, "b_back"),
            edge("a", Port::Out, "join"),
            edge("b_back", Port::Out, "join"),
            edge("join", Port::Out, "after"),
        ],
    ));
    let (report, _, context) = f.run_with(workflow, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Failed);
    let join = report.node(&n("join")).unwrap();
    assert_eq!(join.status, NodeStatus::Failed);
    let error = join.error.as_ref().unwrap();
    assert_eq!(error.code, "context-conflict");
    assert!(error.message.contains("CMS, SHOP"), "{}", error.message);
    assert_eq!(status(&report, "after"), NodeStatus::Skipped);
    assert_eq!(f.ran(), Vec::<String>::new());
    // The run keeps its initial project.
    assert_eq!(project_key(&f, &context), "SHOP");
}

#[test]
fn branches_that_end_apart_on_different_projects_leave_the_context_unchanged() {
    let mut f = Fixture::new();
    let to_cms = select_project(&mut f, "Select CMS", "CMS");
    let also_cms = select_project(&mut f, "Select CMS too", "CMS");
    let back = select_project(&mut f, "Back to SHOP", "SHOP");
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node("fork", NodeKind::Parallel),
            action("a", to_cms),
            action("b", also_cms),
            action("b_back", back),
        ],
        vec![
            edge("start", Port::Out, "fork"),
            edge("fork", Port::Out, "a"),
            edge("fork", Port::Out, "b"),
            edge("b", Port::Out, "b_back"),
        ],
    ));
    let (report, _, context) = f.run_with(workflow, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Succeeded);
    assert!(
        report
            .context_conflict
            .as_deref()
            .is_some_and(|m| m.contains("different projects"))
    );
    assert_eq!(project_key(&f, &context), "SHOP");
}

#[test]
fn a_project_change_applies_to_the_nodes_after_it() {
    let mut f = Fixture::new();
    let to_cms = select_project(&mut f, "Select CMS", "CMS");
    let saw = f.probe("saw CMS", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            is_project("before", "SHOP"),
            action("select", to_cms),
            is_project("check", "CMS"),
            action("saw", saw),
        ],
        vec![
            edge("start", Port::Out, "before"),
            edge("before", Port::True, "select"),
            edge("select", Port::Out, "check"),
            edge("check", Port::True, "saw"),
        ],
    ));
    let (report, _, context) = f.run_with(workflow, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(f.ran(), ["saw CMS"]);
    assert_eq!(project_key(&f, &context), "CMS");
}

#[test]
fn failures_are_routed_continued_or_stop_the_workflow() {
    let mut f = Fixture::new();
    let fail = f.probe("fail", &[]);
    let handler = f.probe("handler", &[]);
    let next = f.probe("next", &[]);

    // Error port connected: handled, the workflow succeeds.
    let routed = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            action("risky", fail),
            action("handler", handler),
        ],
        vec![
            edge("start", Port::Out, "risky"),
            edge("risky", Port::Error, "handler"),
        ],
    ));
    let report = f.run(routed);
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(status(&report, "risky"), NodeStatus::Failed);
    assert_eq!(
        report
            .node(&n("risky"))
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .code,
        "action.failed"
    );
    assert_eq!(status(&report, "handler"), NodeStatus::Succeeded);

    // Continue: on through `out`; continuing is the user's choice, so the
    // run succeeds and the node shows the failure.
    let continued = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            with_policy(
                action("risky", fail),
                ErrorPolicy {
                    on_error: OnError::Continue,
                    ..ErrorPolicy::default()
                },
            ),
            action("next", next),
        ],
        vec![
            edge("start", Port::Out, "risky"),
            edge("risky", Port::Out, "next"),
        ],
    ));
    let report = f.run(continued);
    assert_eq!(status(&report, "next"), NodeStatus::Succeeded);

    // Stop: the workflow fails and nothing after it runs.
    let stopped = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            action("risky", fail),
            action("next", next),
        ],
        vec![
            edge("start", Port::Out, "risky"),
            edge("risky", Port::Out, "next"),
        ],
    ));
    f.log.lock().unwrap().clear();
    let report = f.run(stopped);
    assert_eq!(report.status, RunStatus::Failed);
    assert_eq!(status(&report, "next"), NodeStatus::Skipped);
    assert_eq!(f.ran(), ["fail"]);
}

#[test]
fn retries_with_backoff_only_where_safe() {
    let mut f = Fixture::new();
    let flaky = f.probe("flaky", &[("fail_times", ParameterValue::Integer(2))]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            with_policy(
                action("flaky", flaky),
                ErrorPolicy {
                    retry: RetryPolicy {
                        attempts: 2,
                        backoff_ms: 10,
                    },
                    ..ErrorPolicy::default()
                },
            ),
        ],
        vec![edge("start", Port::Out, "flaky")],
    ));
    let (report, events, _) = f.run_with(workflow, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(report.node(&n("flaky")).unwrap().attempts, 3);
    let waits: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            ExecutionEvent::Retrying { wait_ms, .. } => Some(*wait_ms),
            _ => None,
        })
        .collect();
    assert_eq!(waits, [10, 20]);
    assert_eq!(f.calls.load(Ordering::SeqCst), 3);

    // Retrying an action that is not declared safe is rejected.
    let select = f.add(select_type(), "Select", crate::ActionParameters::new());
    let unsafe_retry = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            with_policy(
                action("select", select),
                ErrorPolicy {
                    retry: RetryPolicy {
                        attempts: 1,
                        backoff_ms: 0,
                    },
                    ..ErrorPolicy::default()
                },
            ),
        ],
        vec![edge("start", Port::Out, "select")],
    ));
    assert!(
        f.issues(unsafe_retry)
            .iter()
            .any(|i| i.message.contains("cannot be retried safely"))
    );
}

#[test]
fn late_results_are_discarded_as_timeouts() {
    let mut f = Fixture::new();
    let slow = f.probe("slow", &[("sleep_ms", ParameterValue::Integer(60))]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            with_policy(
                action("slow", slow),
                ErrorPolicy {
                    timeout_ms: Some(10),
                    ..ErrorPolicy::default()
                },
            ),
        ],
        vec![edge("start", Port::Out, "slow")],
    ));
    let report = f.run(workflow);
    assert_eq!(report.status, RunStatus::Failed);
    let slow = report.node(&n("slow")).unwrap();
    assert_eq!(slow.error.as_ref().unwrap().code, "timeout");
    assert!(slow.output.is_empty());
}

#[test]
fn cancellation_interrupts_delays_and_skips_the_rest() {
    let mut f = Fixture::new();
    let after = f.probe("after", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node("wait", NodeKind::Delay { ms: 5_000 }),
            action("after", after),
        ],
        vec![
            edge("start", Port::Out, "wait"),
            edge("wait", Port::Out, "after"),
        ],
    ));
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        trigger.cancel();
    });
    let started = Instant::now();
    let (report, _, _) = f.run_with(workflow, &cancel);
    canceller.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(report.status, RunStatus::Cancelled);
    assert_eq!(status(&report, "wait"), NodeStatus::Cancelled);
    assert_eq!(status(&report, "after"), NodeStatus::Cancelled);
    assert_eq!(f.ran(), Vec::<String>::new());
}

#[test]
fn loops_run_another_workflow_a_bounded_number_of_times() {
    let mut f = Fixture::new();
    let tick = f.probe("tick", &[]);
    let mut inner = Workflow::new("Inner").step(tick, OnFailure::Stop);
    inner.steps[0].condition = StepCondition::Always;
    let inner_id = inner.id;
    f.profile.workflows.push(inner);
    let outer = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node(
                "repeat",
                NodeKind::Loop {
                    workflow: inner_id,
                    times: 3,
                },
            ),
        ],
        vec![edge("start", Port::Out, "repeat")],
    ));
    let report = f.run(outer);
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(f.ran(), ["tick", "tick", "tick"]);
    assert_eq!(report.node(&n("repeat")).unwrap().attempts, 3);

    // Too many iterations, and a workflow that loops into itself.
    let selfish = f.workflow(WorkflowGraph::default());
    let graph_with_loop = graph(
        vec![
            node("start", NodeKind::Trigger),
            node(
                "again",
                NodeKind::Loop {
                    workflow: selfish,
                    times: MAX_LOOP_ITERATIONS + 1,
                },
            ),
        ],
        vec![edge("start", Port::Out, "again")],
    );
    f.profile.workflows.last_mut().unwrap().graph = Some(graph_with_loop);
    let issues = f.issues(selfish);
    assert!(issues.iter().any(|i| i.message.contains("inside itself")));
    assert!(issues.iter().any(|i| i.message.contains("between 1 and")));
}

#[test]
fn validation_reports_problems_on_the_affected_nodes() {
    let mut f = Fixture::new();
    let a = f.probe("a", &[]);
    let mut referencing = action("b", a);
    if let NodeKind::Action { mappings, .. } = &mut referencing.kind {
        mappings.insert(
            "message".into(),
            Mapping::Output {
                reference: Reference::parse("steps.later.output.value").unwrap(),
                default: None,
            },
        );
    }
    let cyclic = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            action("x", a),
            action("y", a),
        ],
        vec![
            edge("start", Port::Out, "x"),
            edge("x", Port::Out, "y"),
            edge("y", Port::Out, "x"),
        ],
    ));
    assert!(f.issues(cyclic).iter().any(|i| i.message.contains("cycle")));

    let broken = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            referencing,
            action("later", a),
            node(
                "check",
                NodeKind::Condition {
                    predicate: Predicate::ActiveProject { key: "NOPE".into() },
                },
            ),
            node("orphan", NodeKind::Stop),
        ],
        vec![
            edge("start", Port::Out, "b"),
            edge("b", Port::Out, "later"),
            edge("later", Port::Out, "check"),
            edge("start", Port::True, "orphan"),
        ],
    ));
    let issues = f.issues(broken);
    let on = |id: &str| {
        issues
            .iter()
            .filter(|i| i.node.as_ref().map(NodeId::as_str) == Some(id))
            .count()
    };
    assert!(on("start") >= 1, "{issues:?}"); // no True port
    assert!(
        issues
            .iter()
            .any(|i| i.node == Some(n("b")) && i.message.contains("does not run before"))
    );
    assert!(
        issues
            .iter()
            .any(|i| i.node == Some(n("check")) && i.message.contains("no project `NOPE`"))
    );

    // Without a start node.
    let headless = f.workflow(graph(vec![action("x", a)], vec![]));
    assert!(
        f.issues(headless)
            .iter()
            .any(|i| i.message.contains("start node"))
    );
}

#[test]
fn nodes_of_unavailable_action_types_block_the_run() {
    let mut f = Fixture::new();
    let gone = f.add(
        ActionTypeId::new("gone.thing").unwrap(),
        "Gone",
        crate::ActionParameters::new(),
    );
    let workflow = f.workflow(graph(
        vec![node("start", NodeKind::Trigger), action("gone", gone)],
        vec![edge("start", Port::Out, "gone")],
    ));
    assert!(f.issues(workflow).iter().any(|i| i.node == Some(n("gone"))));
    let mut context = ActiveProjectContext::new(&f.profile);
    let result = f.engine.run_graph(
        &f.profile,
        &MachineConfig::new(Os::current()),
        &mut context,
        workflow,
        &CancellationToken::new(),
        &|_| {},
    );
    assert!(matches!(
        result,
        Err(crate::EngineError::InvalidWorkflow(_))
    ));
    assert_eq!(f.ran(), Vec::<String>::new());
}

#[test]
fn the_project_context_is_captured_and_changed_only_by_actions_in_order() {
    let mut f = Fixture::new();
    let cms = f.profile.projects[1].id;
    let select = f.add(
        select_type(),
        "Select CMS",
        crate::ActionParameters::new().with("project", ParameterValue::Project(cms)),
    );
    let then = f.probe("on cms", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node(
                "is_shop",
                NodeKind::Condition {
                    predicate: Predicate::ActiveProject { key: "SHOP".into() },
                },
            ),
            action("select", select),
            node(
                "is_cms",
                NodeKind::Condition {
                    predicate: Predicate::ActiveProject { key: "CMS".into() },
                },
            ),
            action("then", then),
        ],
        vec![
            edge("start", Port::Out, "is_shop"),
            edge("is_shop", Port::True, "select"),
            edge("select", Port::Out, "is_cms"),
            edge("is_cms", Port::True, "then"),
        ],
    ));
    assert!(f.issues(workflow).is_empty(), "{:?}", f.issues(workflow));
    let (report, _, context) = f.run_with(workflow, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(status(&report, "then"), NodeStatus::Succeeded);
    assert_eq!(context.project_id(), Some(cms));
}

#[test]
fn legacy_workflows_convert_and_keep_their_behaviour() {
    for first_fails in [false, true] {
        let mut f = Fixture::new();
        let first = f.probe(if first_fails { "fail" } else { "first" }, &[]);
        let on_success = f.probe("on success", &[]);
        let on_failure = f.probe("on failure", &[]);
        let last = f.probe("last", &[]);
        let mut legacy = Workflow::new("Legacy");
        legacy.steps = vec![
            WorkflowStep::new(first, OnFailure::Continue),
            WorkflowStep {
                condition: StepCondition::PreviousSucceeded,
                ..WorkflowStep::new(on_success, OnFailure::Stop)
            },
            WorkflowStep {
                condition: StepCondition::PreviousFailed,
                ..WorkflowStep::new(on_failure, OnFailure::Stop)
            },
            WorkflowStep::new(last, OnFailure::Stop),
        ];
        let legacy_id = legacy.id;
        f.profile.workflows.push(legacy.clone());

        // The sequential engine, as before.
        let mut context = ActiveProjectContext::new(&f.profile);
        f.engine
            .run_workflow(
                &f.profile,
                &MachineConfig::new(Os::current()),
                &mut context,
                legacy_id,
            )
            .unwrap();
        let sequential = f.ran();
        f.log.lock().unwrap().clear();

        let conversion = from_steps(&legacy, &f.profile);
        // Step 3 checks step 2 in the graph but "the last step that ran" in
        // the steps: noted for review.
        assert_eq!(conversion.notes.len(), 1, "{:?}", conversion.notes);
        let converted = f.workflow(conversion.graph);
        assert!(
            WorkflowGraph::is_runnable(&f.issues(converted)),
            "{:?}",
            f.issues(converted)
        );
        f.run(converted);
        let graph_run = f.ran();
        if first_fails {
            // Steps: first fails, step 2 skipped, step 3 checks step 1 (the
            // last that ran) and runs. Graph: step 3 checks step 2, which
            // was skipped, so it runs too.
            assert_eq!(sequential, ["fail", "on failure", "last"]);
        } else {
            assert_eq!(sequential, ["first", "on success", "last"]);
        }
        assert_eq!(graph_run, sequential);
    }
}

#[test]
fn graphs_round_trip_and_legacy_workflows_serialize_unchanged() {
    let mut f = Fixture::new();
    let a = f.probe("a", &[]);
    let mut node_a = with_policy(
        action("a", a),
        ErrorPolicy {
            on_error: OnError::Continue,
            retry: RetryPolicy {
                attempts: 1,
                backoff_ms: 100,
            },
            timeout_ms: Some(500),
        },
    );
    node_a.position = Position { x: 10.0, y: 20.0 };
    let original = graph(
        vec![node("start", NodeKind::Trigger), node_a],
        vec![edge("start", Port::Out, "a")],
    );
    let json = serde_json::to_string(&original).unwrap();
    assert_eq!(
        serde_json::from_str::<WorkflowGraph>(&json).unwrap(),
        original
    );
    assert!(json.contains(r#""type":"action""#));

    let legacy = Workflow::new("Plain").step(a, OnFailure::Stop);
    let json = serde_json::to_value(&legacy).unwrap();
    assert!(json.get("graph").is_none());
    let restored: Workflow = serde_json::from_value(json).unwrap();
    assert_eq!(restored.graph, None);

    // Node ids are restricted to what references can name.
    assert!(serde_json::from_str::<NodeId>(r#""Bad Id""#).is_err());
}

/// Cancels `token` after `ms`, from another thread.
fn cancel_after(token: &CancellationToken, ms: u64) -> std::thread::JoinHandle<()> {
    let token = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(ms));
        token.cancel();
    })
}

#[test]
fn cancellation_reaches_cooperative_actions_while_they_run() {
    let mut f = Fixture::new();
    let wait = f.probe("long", &[("wait_ms", ParameterValue::Integer(30_000))]);
    let after = f.probe("after", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            action("long", wait),
            action("after", after),
        ],
        vec![
            edge("start", Port::Out, "long"),
            edge("long", Port::Out, "after"),
        ],
    ));
    let cancel = CancellationToken::new();
    let canceller = cancel_after(&cancel, 100);
    let begun = std::time::Instant::now();
    let (report, events, _) = f.run_with(workflow, &cancel);
    canceller.join().unwrap();
    assert!(
        begun.elapsed() < Duration::from_secs(5),
        "{:?}",
        begun.elapsed()
    );
    assert_eq!(report.status, RunStatus::Cancelled);
    let long = report.node(&n("long")).unwrap();
    assert_eq!(long.status, NodeStatus::Cancelled);
    assert_eq!(long.error.as_ref().unwrap().code, "cancelled");
    assert_eq!(status(&report, "after"), NodeStatus::Cancelled);
    // Progress reported by the action arrived as an event.
    assert!(events.iter().any(|e| matches!(
        e,
        ExecutionEvent::NodeProgress { node, progress } if node == &n("long") && progress.message == "waiting"
    )));
}

#[test]
fn cancellation_during_retry_backoff_is_not_a_failure() {
    let mut f = Fixture::new();
    let flaky = f.probe("flaky", &[("fail_times", ParameterValue::Integer(5))]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            with_policy(
                action("flaky", flaky),
                ErrorPolicy {
                    retry: RetryPolicy {
                        attempts: 3,
                        backoff_ms: 30_000,
                    },
                    ..ErrorPolicy::default()
                },
            ),
        ],
        vec![edge("start", Port::Out, "flaky")],
    ));
    let cancel = CancellationToken::new();
    let canceller = cancel_after(&cancel, 100);
    let begun = std::time::Instant::now();
    let report = f.run_with(workflow, &cancel).0;
    canceller.join().unwrap();
    assert!(begun.elapsed() < Duration::from_secs(5));
    assert_eq!(report.status, RunStatus::Cancelled);
    let flaky = report.node(&n("flaky")).unwrap();
    assert_eq!(flaky.status, NodeStatus::Cancelled);
    assert_eq!(flaky.attempts, 1);
    assert_eq!(flaky.error.as_ref().unwrap().code, "cancelled");
}

#[test]
fn cooperative_actions_stop_at_their_timeout_and_report_a_timeout() {
    let mut f = Fixture::new();
    let wait = f.probe("long", &[("wait_ms", ParameterValue::Integer(30_000))]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            with_policy(
                action("long", wait),
                ErrorPolicy {
                    timeout_ms: Some(100),
                    ..ErrorPolicy::default()
                },
            ),
        ],
        vec![edge("start", Port::Out, "long")],
    ));
    let begun = std::time::Instant::now();
    let report = f.run(workflow);
    assert!(begun.elapsed() < Duration::from_secs(5));
    assert_eq!(report.status, RunStatus::Failed);
    let long = report.node(&n("long")).unwrap();
    assert_eq!(long.status, NodeStatus::Failed);
    assert_eq!(long.error.as_ref().unwrap().code, "timeout");
}

#[test]
fn actions_that_finish_despite_a_cancellation_keep_their_result() {
    let mut f = Fixture::new();
    // Not cooperative: sleeps through the cancellation and succeeds.
    let busy = f.probe("busy", &[("sleep_ms", ParameterValue::Integer(200))]);
    let after = f.probe("after", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            action("busy", busy),
            action("after", after),
        ],
        vec![
            edge("start", Port::Out, "busy"),
            edge("busy", Port::Out, "after"),
        ],
    ));
    let cancel = CancellationToken::new();
    let canceller = cancel_after(&cancel, 50);
    let report = f.run_with(workflow, &cancel).0;
    canceller.join().unwrap();
    assert_eq!(status(&report, "busy"), NodeStatus::Succeeded);
    assert_eq!(status(&report, "after"), NodeStatus::Cancelled);
    assert_eq!(report.status, RunStatus::Cancelled);
}

#[test]
fn saving_and_loading_keeps_positions_exactly() {
    let mut f = Fixture::new();
    let probe = f.probe("p", &[]);
    let mut stored = graph(
        vec![node("start", NodeKind::Trigger), action("p", probe)],
        vec![edge("start", Port::Out, "p")],
    );
    stored.nodes[1].position = Position {
        x: 412.5,
        y: -87.25,
    };
    let json = serde_json::to_string(&stored).unwrap();
    let loaded: WorkflowGraph = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, stored);
    assert_eq!(loaded.nodes[1].position.x, 412.5);
}

#[test]
fn node_time_limits_never_exceed_the_maximum() {
    use super::executor::node_timeout;
    let max = Duration::from_millis(u64::from(MAX_TIMEOUT_MS));
    assert_eq!(node_timeout(None, 30_000), Duration::from_secs(30));
    assert_eq!(node_timeout(Some(500), 30_000), Duration::from_millis(500));
    // An action type that declares more is still bounded in a workflow.
    assert_eq!(node_timeout(None, 3_610_000), max);
    assert_eq!(node_timeout(Some(u32::MAX), 1_000), max);
}

// Sub-workflows ------------------------------------------------------------

fn templates(values: &[(&str, &str)]) -> BTreeMap<String, Mapping> {
    values
        .iter()
        .map(|(key, template)| {
            (
                (*key).to_owned(),
                Mapping::Template {
                    template: (*template).to_owned(),
                },
            )
        })
        .collect()
}

fn call(id: &str, workflow: WorkflowId, inputs: &[(&str, &str)]) -> Node {
    node(
        id,
        NodeKind::Call {
            workflow,
            inputs: templates(inputs),
            policy: ErrorPolicy::default(),
        },
    )
}

fn output_node(id: &str, values: &[(&str, &str)]) -> Node {
    node(
        id,
        NodeKind::Output {
            values: templates(values),
        },
    )
}

fn mapped(mut node: Node, key: &str, template: &str) -> Node {
    if let NodeKind::Action { mappings, .. } = &mut node.kind {
        mappings.insert(
            key.into(),
            Mapping::Template {
                template: template.into(),
            },
        );
    }
    node
}

impl Fixture {
    /// Adds a graph workflow with declared inputs and outputs.
    fn declared(
        &mut self,
        name: &str,
        graph: WorkflowGraph,
        inputs: Vec<WorkflowInput>,
        outputs: Vec<WorkflowOutput>,
    ) -> WorkflowId {
        let mut workflow = Workflow::new(name);
        workflow.graph = Some(graph);
        workflow.inputs = inputs;
        workflow.outputs = outputs;
        let id = workflow.id;
        self.profile.workflows.push(workflow);
        id
    }

    fn workflow_mut(&mut self, id: WorkflowId) -> &mut Workflow {
        self.profile
            .workflows
            .iter_mut()
            .find(|w| w.id == id)
            .unwrap()
    }

    fn errors(&self, workflow: WorkflowId) -> Vec<String> {
        self.issues(workflow)
            .into_iter()
            .filter(|i| i.severity == IssueSeverity::Error)
            .map(|i| i.message)
            .collect()
    }

    /// `Greet`: takes `name` (required) and `times` (whole number, default
    /// 2); says hello and returns `greeting` and `times`.
    fn greeter(&mut self) -> WorkflowId {
        let hello = self.probe("hello", &[]);
        let required = WorkflowInput {
            required: true,
            ..WorkflowInput::new("name", ValueKind::Text)
        };
        let times = WorkflowInput {
            default: Some("2".into()),
            ..WorkflowInput::new("times", ValueKind::Integer)
        };
        self.declared(
            "Greet",
            graph(
                vec![
                    node("start", NodeKind::Trigger),
                    mapped(
                        action("hello", hello),
                        "message",
                        "hello {{ steps.start.output.name }}",
                    ),
                    output_node(
                        "done",
                        &[
                            ("greeting", "{{ steps.hello.output.value }}"),
                            ("times", " {{ steps.start.output.times }} "),
                        ],
                    ),
                ],
                vec![
                    edge("start", Port::Out, "hello"),
                    edge("hello", Port::Out, "done"),
                ],
            ),
            vec![required, times],
            vec![
                WorkflowOutput::new("greeting", ValueKind::Text),
                WorkflowOutput::new("times", ValueKind::Integer),
            ],
        )
    }

    /// A workflow whose only action waits 30 s cooperatively.
    fn long_workflow(&mut self) -> WorkflowId {
        let long = self.probe("long", &[("wait_ms", ParameterValue::Integer(30_000))]);
        self.declared(
            "Long",
            graph(
                vec![node("start", NodeKind::Trigger), action("long", long)],
                vec![edge("start", Port::Out, "long")],
            ),
            vec![],
            vec![],
        )
    }
}

#[test]
fn sub_workflows_take_typed_inputs_and_return_declared_outputs() {
    let mut f = Fixture::new();
    let greet = f.greeter();
    let world = f.probe("world", &[]);
    let after = f.probe("after", &[]);
    let parent = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            action("world", world),
            call(
                "greet",
                greet,
                &[("name", "{{ steps.world.output.value }}")],
            ),
            mapped(
                action("after", after),
                "message",
                "{{ steps.greet.output.greeting }} x{{ steps.greet.output.times }}",
            ),
        ],
        vec![
            edge("start", Port::Out, "world"),
            edge("world", Port::Out, "greet"),
            edge("greet", Port::Out, "after"),
        ],
    ));
    assert!(f.issues(greet).is_empty(), "{:?}", f.issues(greet));
    assert!(f.issues(parent).is_empty(), "{:?}", f.issues(parent));
    let (report, events, _) = f.run_with(parent, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Succeeded, "{report:?}");
    assert_eq!(f.ran(), ["world", "hello world", "hello world x2"]);
    let greeted = report.node(&n("greet")).unwrap();
    assert_eq!(greeted.workflow, Some(greet));
    assert_eq!(greeted.attempts, 1);
    // Declared outputs only, converted to their kinds (the default `2`
    // passed through, trimmed).
    assert_eq!(
        greeted.output,
        BTreeMap::from([
            ("greeting".to_owned(), "hello world".to_owned()),
            ("times".to_owned(), "2".to_owned()),
        ])
    );
    // The callee's own nodes are not reported as nodes of the parent.
    assert!(report.node(&n("hello")).is_none());
    assert!(events.iter().all(|e| match e {
        ExecutionEvent::NodeStarted { node } => node != &n("hello"),
        _ => true,
    }));
}

#[test]
fn sub_workflow_inputs_are_converted_and_checked_at_run_time() {
    let mut f = Fixture::new();
    let greet = f.greeter();
    let word = f.probe("many", &[]);
    let parent = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            action("word", word),
            call(
                "greet",
                greet,
                &[
                    ("name", "x"),
                    // Not a whole number.
                    ("times", "{{ steps.word.output.value }}"),
                ],
            ),
        ],
        vec![
            edge("start", Port::Out, "word"),
            edge("word", Port::Out, "greet"),
        ],
    ));
    let report = f.run(parent);
    assert_eq!(report.status, RunStatus::Failed);
    let error = report.node(&n("greet")).unwrap().error.clone().unwrap();
    assert_eq!(error.code, "mapping");
    assert!(error.message.contains("whole number"), "{}", error.message);
    // The callee never ran.
    assert_eq!(f.ran(), ["many"]);
}

#[test]
fn sub_workflow_failures_propagate_with_a_stable_code() {
    let mut f = Fixture::new();
    let broken = f.probe("fail", &[]);
    let callee = f.declared(
        "Broken",
        graph(
            vec![node("start", NodeKind::Trigger), action("boom", broken)],
            vec![edge("start", Port::Out, "boom")],
        ),
        vec![],
        vec![],
    );
    let handler = f.probe("handled", &[]);
    // Unhandled: the parent fails.
    let unhandled = f.workflow(graph(
        vec![node("start", NodeKind::Trigger), call("sub", callee, &[])],
        vec![edge("start", Port::Out, "sub")],
    ));
    let report = f.run(unhandled);
    assert_eq!(report.status, RunStatus::Failed);
    let sub = report.node(&n("sub")).unwrap();
    assert_eq!(sub.status, NodeStatus::Failed);
    let error = sub.error.as_ref().unwrap();
    assert_eq!(error.code, "subworkflow.failed");
    assert!(
        error.message.contains("Broken failed: boom"),
        "{}",
        error.message
    );
    // Handled through the error port.
    let routed = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            call("sub", callee, &[]),
            action("handler", handler),
        ],
        vec![
            edge("start", Port::Out, "sub"),
            edge("sub", Port::Error, "handler"),
        ],
    ));
    let report = f.run(routed);
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(status(&report, "sub"), NodeStatus::Failed);
    assert_eq!(status(&report, "handler"), NodeStatus::Succeeded);
}

#[test]
fn cancellation_reaches_a_running_sub_workflow() {
    let mut f = Fixture::new();
    let callee = f.long_workflow();
    let after = f.probe("after", &[]);
    let parent = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            call("sub", callee, &[]),
            action("after", after),
        ],
        vec![
            edge("start", Port::Out, "sub"),
            edge("sub", Port::Out, "after"),
        ],
    ));
    let cancel = CancellationToken::new();
    let canceller = cancel_after(&cancel, 100);
    let begun = Instant::now();
    let (report, _, _) = f.run_with(parent, &cancel);
    canceller.join().unwrap();
    assert!(
        begun.elapsed() < Duration::from_secs(5),
        "{:?}",
        begun.elapsed()
    );
    assert_eq!(report.status, RunStatus::Cancelled);
    let sub = report.node(&n("sub")).unwrap();
    assert_eq!(sub.status, NodeStatus::Cancelled);
    assert_eq!(sub.error.as_ref().unwrap().code, "cancelled");
    assert_eq!(status(&report, "after"), NodeStatus::Cancelled);
    assert_eq!(f.ran(), ["long"]);
}

#[test]
fn sub_workflows_stop_at_their_timeout() {
    let mut f = Fixture::new();
    let callee = f.long_workflow();
    let mut sub = call("sub", callee, &[]);
    if let NodeKind::Call { policy, .. } = &mut sub.kind {
        policy.timeout_ms = Some(100);
    }
    let parent = f.workflow(graph(
        vec![node("start", NodeKind::Trigger), sub],
        vec![edge("start", Port::Out, "sub")],
    ));
    let begun = Instant::now();
    let report = f.run(parent);
    assert!(
        begun.elapsed() < Duration::from_secs(5),
        "{:?}",
        begun.elapsed()
    );
    assert_eq!(report.status, RunStatus::Failed);
    let sub = report.node(&n("sub")).unwrap();
    assert_eq!(sub.status, NodeStatus::Failed);
    assert_eq!(sub.error.as_ref().unwrap().code, "timeout");
}

#[test]
fn sub_workflows_run_in_the_callers_project_context() {
    let mut f = Fixture::new();
    let select = select_project(&mut f, "Select CMS", "CMS");
    let inside = f.probe("in cms", &[]);
    let callee = f.declared(
        "Check",
        graph(
            vec![
                node("start", NodeKind::Trigger),
                is_project("cms", "CMS"),
                action("inside", inside),
            ],
            vec![
                edge("start", Port::Out, "cms"),
                edge("cms", Port::True, "inside"),
            ],
        ),
        vec![],
        vec![],
    );
    let parent = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            action("select", select),
            call("sub", callee, &[]),
        ],
        vec![
            edge("start", Port::Out, "select"),
            edge("select", Port::Out, "sub"),
        ],
    ));
    let (report, _, context) = f.run_with(parent, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(f.ran(), ["in cms"]);
    assert_eq!(project_key(&f, &context), "CMS");
}

#[test]
fn sub_workflow_validation_reports_inputs_outputs_and_retries() {
    let mut f = Fixture::new();
    let greet = f.greeter();
    let missing = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            call("greet", greet, &[("nickname", "x")]),
            output_node("out", &[("undeclared", "x")]),
        ],
        vec![
            edge("start", Port::Out, "greet"),
            edge("greet", Port::Out, "out"),
        ],
    ));
    let errors = f.errors(missing);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("`nickname` is not an input of Greet")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("needs a value for its input `name`")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("`undeclared` is not an output")),
        "{errors:?}"
    );

    // An output the callee does not declare cannot be referenced.
    let probe = f.probe("p", &[]);
    let referencing = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            call("greet", greet, &[("name", "x")]),
            mapped(
                action("use", probe),
                "message",
                "{{ steps.greet.output.nope }}",
            ),
        ],
        vec![
            edge("start", Port::Out, "greet"),
            edge("greet", Port::Out, "use"),
        ],
    ));
    let errors = f.errors(referencing);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("provides no output `nope`")),
        "{errors:?}"
    );

    // Retries are refused; a Loop cannot run a workflow that needs inputs.
    let mut retried = call("again", greet, &[("name", "x")]);
    if let NodeKind::Call { policy, .. } = &mut retried.kind {
        policy.retry.attempts = 1;
    }
    let looping = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            retried,
            node(
                "repeat",
                NodeKind::Loop {
                    workflow: greet,
                    times: 2,
                },
            ),
        ],
        vec![
            edge("start", Port::Out, "again"),
            edge("again", Port::Out, "repeat"),
        ],
    ));
    let errors = f.errors(looping);
    assert!(
        errors.iter().any(|e| e.contains("cannot be retried")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("run it with a Sub-workflow node")),
        "{errors:?}"
    );

    // A workflow that does not exist.
    let ghost = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            call("ghost", WorkflowId::new(), &[]),
        ],
        vec![edge("start", Port::Out, "ghost")],
    ));
    assert!(
        f.errors(ghost)
            .iter()
            .any(|e| e.contains("does not exist in this profile"))
    );
}

#[test]
fn workflow_declarations_are_checked() {
    let mut f = Fixture::new();
    // Declarations: keys, duplicates and defaults of the declared kind.
    let bad = f.declared(
        "Bad",
        graph(vec![node("start", NodeKind::Trigger)], vec![]),
        vec![
            WorkflowInput::new("a b", ValueKind::Text),
            WorkflowInput::new("x", ValueKind::Text),
            WorkflowInput {
                default: Some("many".into()),
                ..WorkflowInput::new("x", ValueKind::Integer)
            },
        ],
        vec![],
    );
    let errors = f.errors(bad);
    assert!(
        errors.iter().any(|e| e.contains("not a valid input key")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.contains("declared twice")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.contains("whole number")),
        "{errors:?}"
    );
}

#[test]
fn recursion_is_rejected_directly_indirectly_and_through_loops() {
    let mut f = Fixture::new();
    // Direct: a workflow that calls itself.
    let selfish = f.workflow(WorkflowGraph::default());
    f.workflow_mut(selfish).graph = Some(graph(
        vec![node("start", NodeKind::Trigger), call("me", selfish, &[])],
        vec![edge("start", Port::Out, "me")],
    ));
    assert!(
        f.errors(selfish)
            .iter()
            .any(|e| e.contains("inside itself"))
    );
    assert!(f.engine.check_runnable(&f.profile, selfish).is_err());

    // Indirect: a calls b, b loops over a.
    let a = f.workflow(WorkflowGraph::default());
    let b = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node(
                "repeat",
                NodeKind::Loop {
                    workflow: a,
                    times: 2,
                },
            ),
        ],
        vec![edge("start", Port::Out, "repeat")],
    ));
    f.workflow_mut(a).graph = Some(graph(
        vec![node("start", NodeKind::Trigger), call("b", b, &[])],
        vec![edge("start", Port::Out, "b")],
    ));
    assert!(f.errors(a).iter().any(|e| e.contains("inside itself")));
    assert!(f.errors(b).iter().any(|e| e.contains("inside itself")));
    assert!(f.engine.check_runnable(&f.profile, a).is_err());
}

#[test]
fn nesting_depth_is_limited() {
    let mut f = Fixture::new();
    let leaf = f.probe("leaf", &[]);
    // The first workflow runs an action; each next one calls the one before.
    let mut top = f.workflow(graph(
        vec![node("start", NodeKind::Trigger), action("leaf", leaf)],
        vec![edge("start", Port::Out, "leaf")],
    ));
    for _ in 1..MAX_NESTING {
        top = f.workflow(graph(
            vec![node("start", NodeKind::Trigger), call("down", top, &[])],
            vec![edge("start", Port::Out, "down")],
        ));
    }
    // MAX_NESTING levels run.
    assert!(f.errors(top).is_empty(), "{:?}", f.errors(top));
    assert_eq!(f.run(top).status, RunStatus::Succeeded);
    assert_eq!(f.ran(), ["leaf"]);
    // One more is refused.
    let too_deep = f.workflow(graph(
        vec![node("start", NodeKind::Trigger), call("down", top, &[])],
        vec![edge("start", Port::Out, "down")],
    ));
    assert!(
        f.errors(too_deep)
            .iter()
            .any(|e| e.contains(&format!("at most {MAX_NESTING} levels"))),
        "{:?}",
        f.errors(too_deep)
    );
}

#[test]
fn sensitive_values_need_sensitive_inputs_and_outputs() {
    let mut f = Fixture::new();
    let token = WorkflowInput {
        sensitive: true,
        ..WorkflowInput::new("token", ValueKind::Text)
    };
    let echo = |output: WorkflowOutput| (vec![token.clone()], vec![output]);
    let leaky_graph = graph(
        vec![
            node("start", NodeKind::Trigger),
            output_node("out", &[("echo", "{{ steps.start.output.token }}")]),
        ],
        vec![edge("start", Port::Out, "out")],
    );
    let (inputs, outputs) = echo(WorkflowOutput::new("echo", ValueKind::Text));
    let leaky = f.declared("Leaky", leaky_graph.clone(), inputs, outputs);
    assert!(
        f.errors(leaky)
            .iter()
            .any(|e| e.contains("mark the output sensitive")),
        "{:?}",
        f.errors(leaky)
    );
    let (inputs, outputs) = echo(WorkflowOutput {
        sensitive: true,
        ..WorkflowOutput::new("echo", ValueKind::Text)
    });
    let safe = f.declared("Safe", leaky_graph, inputs, outputs);
    assert!(f.errors(safe).is_empty(), "{:?}", f.errors(safe));
    // A sensitive output passed on to an input that is not sensitive.
    let plain = f.declared(
        "Plain",
        graph(vec![node("start", NodeKind::Trigger)], vec![]),
        vec![WorkflowInput::new("value", ValueKind::Text)],
        vec![],
    );
    let parent = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            call("safe", safe, &[("token", "abc")]),
            call("plain", plain, &[("value", "{{ steps.safe.output.echo }}")]),
        ],
        vec![
            edge("start", Port::Out, "safe"),
            edge("safe", Port::Out, "plain"),
        ],
    ));
    assert!(
        f.errors(parent)
            .iter()
            .any(|e| e.contains("does not mark that input sensitive")),
        "{:?}",
        f.errors(parent)
    );
}

#[test]
fn history_keeps_only_declared_non_sensitive_sub_workflow_outputs() {
    let mut f = Fixture::new();
    let callee = f.declared(
        "Secretive",
        graph(
            vec![
                node("start", NodeKind::Trigger),
                output_node(
                    "out",
                    &[
                        ("public", "visible"),
                        ("secret", "{{ steps.start.output.token }}"),
                    ],
                ),
            ],
            vec![edge("start", Port::Out, "out")],
        ),
        vec![WorkflowInput {
            sensitive: true,
            ..WorkflowInput::new("token", ValueKind::Text)
        }],
        vec![
            WorkflowOutput::new("public", ValueKind::Text),
            WorkflowOutput {
                sensitive: true,
                ..WorkflowOutput::new("secret", ValueKind::Text)
            },
        ],
    );
    assert!(f.errors(callee).is_empty(), "{:?}", f.errors(callee));
    let parent = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            call("sub", callee, &[("token", "hunter2")]),
        ],
        vec![edge("start", Port::Out, "sub")],
    ));
    let (report, events, _) = f.run_with(parent, &CancellationToken::new());
    assert_eq!(report.status, RunStatus::Succeeded);
    let sub = report.node(&n("sub")).unwrap();
    // Later nodes can use every declared output...
    assert_eq!(sub.output["secret"], "hunter2");
    assert_eq!(sub.output["public"], "visible");
    // ...history and live events keep only the non-sensitive ones.
    let safe = crate::safe_report(sub, &f.profile, f.engine.registry());
    assert_eq!(
        safe.output,
        BTreeMap::from([("public".to_owned(), "visible".to_owned())])
    );
    let stored = serde_json::to_string(
        &report
            .nodes
            .iter()
            .map(|node| crate::safe_report(node, &f.profile, f.engine.registry()))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(!stored.contains("hunter2"), "{stored}");
    for event in events {
        let text =
            serde_json::to_string(&crate::safe_event(event, &f.profile, f.engine.registry()))
                .unwrap();
        assert!(!text.contains("hunter2"), "{text}");
    }
}

#[test]
fn workflows_with_required_inputs_do_not_run_on_their_own() {
    let mut f = Fixture::new();
    let greet = f.greeter();
    let error = f.engine.check_runnable(&f.profile, greet).unwrap_err();
    assert!(error.to_string().contains("input `name`"), "{error}");
    // Optional inputs take their defaults when run on their own.
    f.workflow_mut(greet).inputs[0] = WorkflowInput {
        default: Some("you".into()),
        ..WorkflowInput::new("name", ValueKind::Text)
    };
    assert!(f.engine.check_runnable(&f.profile, greet).is_ok());
    assert_eq!(f.run(greet).status, RunStatus::Succeeded);
    assert_eq!(f.ran(), ["hello you"]);
}

#[test]
fn workflows_with_inputs_and_new_nodes_round_trip() {
    let mut f = Fixture::new();
    let greet = f.greeter();
    let workflow = f.profile.workflow(greet).unwrap().clone();
    let json = serde_json::to_value(&workflow).unwrap();
    assert_eq!(json["inputs"][0]["key"], "name");
    assert_eq!(json["inputs"][0]["required"], true);
    assert_eq!(json["outputs"][1]["kind"], "integer");
    assert_eq!(json["graph"]["nodes"][2]["type"], "output");
    assert_eq!(serde_json::from_value::<Workflow>(json).unwrap(), workflow);
    let parent = graph(
        vec![
            node("start", NodeKind::Trigger),
            call("greet", greet, &[("name", "x")]),
        ],
        vec![edge("start", Port::Out, "greet")],
    );
    let json = serde_json::to_value(&parent).unwrap();
    assert_eq!(json["nodes"][1]["type"], "call");
    assert_eq!(json["nodes"][1]["inputs"]["name"]["type"], "template");
    assert_eq!(
        serde_json::from_value::<WorkflowGraph>(json).unwrap(),
        parent
    );
    // Workflows without the new fields do not serialize them.
    let plain = serde_json::to_value(Workflow::new("Plain")).unwrap();
    assert!(plain.get("inputs").is_none() && plain.get("outputs").is_none());
}
