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
        _: &ExecutionContext<'_>,
    ) -> ActionResult {
        let message = parameters.text("message").unwrap_or_default().to_owned();
        self.log.lock().unwrap().push(message.clone());
        if let Some(ms) = parameters.integer("sleep_ms") {
            std::thread::sleep(Duration::from_millis(u64::try_from(ms).unwrap()));
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
                    ],
                    requires_project: false,
                    node: NodeSpec {
                        outputs: vec![OutputSpec {
                            key: "value".into(),
                            label: "Value".into(),
                            kind: ValueKind::Text,
                            always: true,
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

#[test]
fn project_changes_are_not_allowed_in_parallel_branches() {
    let mut f = Fixture::new();
    let select = f.add(select_type(), "Select", crate::ActionParameters::new());
    let other = f.probe("other", &[]);
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node("fork", NodeKind::Parallel),
            action("select", select),
            action("other", other),
            node("join", NodeKind::Join),
        ],
        vec![
            edge("start", Port::Out, "fork"),
            edge("fork", Port::Out, "select"),
            edge("fork", Port::Out, "other"),
            edge("select", Port::Out, "join"),
            edge("other", Port::Out, "join"),
        ],
    ));
    let issues = f.issues(workflow);
    assert!(
        issues.iter().any(|i| i.node == Some(n("select"))
            && i.message.contains("cannot run in parallel branches"))
    );
    assert!(!WorkflowGraph::is_runnable(&issues));
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
