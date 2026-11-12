//! Graph version 2: inline invocations, whole-value variable references and
//! templates over shared and system variables, with the version 1 graphs
//! that must keep loading and running unchanged.

use super::*;
use crate::variables::doubles::MemoryVariables;
use crate::variables::{
    ScopeKey, SystemVariable, VariableDefinition, VariableKey, VariableRef, VariableScope,
    VariableType, VariableValue,
};

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

fn shared(text: &str) -> VariableRef {
    VariableRef::Shared(key(text))
}

fn invoke(id: &str, action_type: crate::ActionTypeId, parameters: crate::ActionParameters) -> Node {
    node(
        id,
        NodeKind::Invoke {
            action_type,
            parameters,
            mappings: BTreeMap::new(),
            policy: ErrorPolicy::default(),
        },
    )
}

fn probe_invoke(id: &str, message: &str) -> Node {
    invoke(
        id,
        probe_type(),
        crate::ActionParameters::new().with("message", ParameterValue::Text(message.into())),
    )
}

fn map(mut node: Node, parameter: &str, mapping: Mapping) -> Node {
    match &mut node.kind {
        NodeKind::Action { mappings, .. } | NodeKind::Invoke { mappings, .. } => {
            mappings.insert(parameter.into(), mapping);
        }
        NodeKind::Output { values } | NodeKind::SetVariable { values } => {
            values.insert(parameter.into(), mapping);
        }
        _ => panic!("no mappings"),
    }
    node
}

fn template(text: &str) -> Mapping {
    Mapping::Template {
        template: text.into(),
    }
}

fn variable(reference: VariableRef) -> Mapping {
    Mapping::Variable {
        reference,
        default: None,
    }
}

fn chain(ids: &[&str]) -> Vec<Edge> {
    ids.windows(2)
        .map(|pair| edge(pair[0], Port::Out, pair[1]))
        .collect()
}

fn errors(issues: &[GraphIssue]) -> Vec<&str> {
    issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Error)
        .map(|i| i.message.as_str())
        .collect()
}

impl Fixture {
    fn run_resolved(
        &self,
        workflow: WorkflowId,
        variables: &MemoryVariables,
        context: &mut ActiveProjectContext,
    ) -> GraphReport {
        let execution = crate::ExecutionId::new();
        self.engine
            .execute_graph_with_services(
                &self.profile,
                &MachineConfig::new(Os::current()),
                context,
                workflow,
                &BTreeMap::new(),
                RunServices {
                    variables: Some(variables),
                    device: None,
                    execution: Some(execution),
                },
                &CancellationToken::new(),
                &|_| {},
            )
            .unwrap()
    }

    fn project(&self, key: &str) -> crate::ProjectId {
        self.profile
            .projects
            .iter()
            .find(|p| p.key.as_str() == key)
            .unwrap()
            .id
    }
}

// --- format versions -------------------------------------------------------

/// A version 1 graph as 0.6.0 stores it: saved instances, `steps.`
/// templates, a whole-output mapping with a default, a condition.
fn legacy_json(first: crate::ActionInstanceId, second: crate::ActionInstanceId) -> String {
    format!(
        r#"{{
  "version": 1,
  "nodes": [
    {{ "id": "start", "type": "trigger", "position": {{ "x": 0.0, "y": 0.0 }} }},
    {{ "id": "first", "type": "action", "action": "{first}", "policy": {{ "onError": "stop", "retry": {{ "attempts": 0, "backoffMs": 0 }} }}, "position": {{ "x": 1.0, "y": 0.0 }} }},
    {{ "id": "check", "type": "condition", "predicate": {{ "type": "notEmpty", "reference": {{ "node": "first", "output": "value" }} }}, "position": {{ "x": 2.0, "y": 0.0 }} }},
    {{ "id": "second", "type": "action", "action": "{second}",
       "mappings": {{
         "message": {{ "type": "template", "template": "got {{{{ steps.first.output.value }}}}" }}
       }},
       "policy": {{ "onError": "stop", "retry": {{ "attempts": 0, "backoffMs": 0 }} }}, "position": {{ "x": 3.0, "y": 0.0 }} }}
  ],
  "edges": [
    {{ "from": "start", "port": "out", "to": "first" }},
    {{ "from": "first", "port": "out", "to": "check" }},
    {{ "from": "check", "port": "true", "to": "second" }}
  ]
}}"#
    )
}

#[test]
fn version_1_graphs_load_run_and_store_unchanged() {
    let mut f = Fixture::new();
    let first = f.probe("alpha", &[]);
    let second = f.probe("unused", &[]);
    let stored = legacy_json(first, second);
    let graph: WorkflowGraph = serde_json::from_str(&stored).unwrap();
    assert_eq!(graph.version, 1);
    assert_eq!(graph.required_version(), 1);
    // Stored again, it is the same document: still version 1.
    assert_eq!(
        serde_json::to_value(&graph).unwrap(),
        serde_json::from_str::<serde_json::Value>(&stored).unwrap()
    );
    let workflow = f.workflow(graph);
    assert!(f.issues(workflow).is_empty(), "{:?}", f.issues(workflow));
    let report = f.run(workflow);
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(f.ran(), ["alpha", "got alpha"]);
    assert_eq!(report.node(&n("second")).unwrap().action, Some(second));
}

#[test]
fn graphs_with_version_2_shapes_are_stored_as_version_2() {
    let mut f = Fixture::new();
    let a = f.probe("alpha", &[]);
    let cases: Vec<(&str, Node)> = vec![
        ("inline node", probe_invoke("first", "inline")),
        (
            "variable mapping",
            map(
                action("first", a),
                "message",
                variable(VariableRef::System(SystemVariable::Platform)),
            ),
        ),
        (
            "shared reference in a template",
            map(
                action("first", a),
                "message",
                template("cd {{ shared.paths.root }}"),
            ),
        ),
    ];
    for (what, first) in cases {
        let mut graph = graph(
            vec![node("start", NodeKind::Trigger), first],
            chain(&["start", "first"]),
        );
        // Even when it was read as version 1 and then edited.
        graph.version = 1;
        assert_eq!(graph.required_version(), 2, "{what}");
        let stored = serde_json::to_value(&graph).unwrap();
        assert_eq!(stored["version"], 2, "{what}");
        let reread: WorkflowGraph = serde_json::from_value(stored).unwrap();
        assert_eq!(reread.version, 2);
        assert_eq!(reread.nodes, graph.nodes);
    }
    // `steps.` templates alone stay version 1.
    let legacy = graph(
        vec![
            node("start", NodeKind::Trigger),
            map(
                action("first", a),
                "message",
                template("{{ steps.start.output.x }}"),
            ),
        ],
        chain(&["start", "first"]),
    );
    assert_eq!(legacy.required_version(), 1);
    assert_eq!(WorkflowGraph::default().version, 1);
}

#[test]
fn unknown_format_versions_are_refused() {
    let mut f = Fixture::new();
    for version in [0, GRAPH_VERSION + 1] {
        let mut g = graph(
            vec![node("start", NodeKind::Trigger), probe_invoke("first", "x")],
            chain(&["start", "first"]),
        );
        g.version = version;
        // A newer version is kept when stored again, not downgraded.
        if version > GRAPH_VERSION {
            assert_eq!(serde_json::to_value(&g).unwrap()["version"], version);
        }
        let workflow = f.workflow(g);
        assert_eq!(
            errors(&f.issues(workflow)),
            [format!("graph format {version} is not supported").as_str()]
        );
    }
}

#[test]
fn the_stored_shapes_match_the_contract() {
    let node = map(
        map(
            invoke("open", select_type(), crate::ActionParameters::new()),
            "project",
            Mapping::Variable {
                reference: shared("projects.favorite"),
                default: None,
            },
        ),
        "unused",
        Mapping::Variable {
            reference: VariableRef::System(SystemVariable::ProjectCurrentId),
            default: Some(VariableValue::String("x".into())),
        },
    );
    let json = serde_json::to_value(&node).unwrap();
    assert_eq!(json["type"], "invoke");
    assert_eq!(json["actionType"], "test.select");
    assert_eq!(
        json["mappings"]["project"],
        serde_json::json!({ "type": "variable", "reference": "shared.projects.favorite" })
    );
    assert_eq!(
        json["mappings"]["unused"],
        serde_json::json!({
            "type": "variable",
            "reference": "system.project.current.id",
            "default": { "type": "string", "value": "x" }
        })
    );
    assert_eq!(serde_json::from_value::<Node>(json).unwrap(), node);
}

// --- validation -------------------------------------------------------------

#[test]
fn inline_nodes_are_validated_like_saved_actions() {
    let mut f = Fixture::new();
    let unknown = invoke(
        "missing",
        crate::ActionTypeId::new("test.nothing").unwrap(),
        crate::ActionParameters::new(),
    );
    let bad_parameter = invoke(
        "bad",
        probe_type(),
        crate::ActionParameters::new().with("colour", ParameterValue::Text("red".into())),
    );
    let mut retried = probe_invoke("retried", "x");
    if let NodeKind::Invoke { policy, .. } = &mut retried.kind {
        policy.retry.attempts = MAX_RETRIES + 1;
        policy.timeout_ms = Some(0);
    }
    let unknown_mapping = map(probe_invoke("mapped", "x"), "colour", template("x"));
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            unknown,
            bad_parameter,
            retried,
            unknown_mapping,
        ],
        chain(&["start", "missing", "bad", "retried", "mapped"]),
    ));
    let issues = f.issues(workflow);
    let on = |id: &str| -> Vec<&str> {
        issues
            .iter()
            .filter(|i| i.node.as_ref() == Some(&n(id)))
            .map(|i| i.message.as_str())
            .collect()
    };
    assert_eq!(on("missing").len(), 1, "{issues:?}");
    assert!(on("missing")[0].starts_with("test.nothing: "), "{issues:?}");
    assert_eq!(on("bad"), ["unknown parameter `colour`"]);
    assert_eq!(
        on("retried"),
        [
            "at most 5 retries",
            "the timeout must be between 1 ms and 10 minutes"
        ]
    );
    assert_eq!(on("mapped"), ["`colour` is not a parameter of Probe"]);
}

#[test]
#[allow(clippy::too_many_lines)] // one graph, one case per node
fn references_are_checked_before_the_run() {
    let mut f = Fixture::new();
    let definitions = [
        VariableDefinition::new(
            key("paths.root"),
            VariableType::String,
            &[VariableScope::Global],
        ),
        VariableDefinition::new(
            key("creds.token"),
            VariableType::SecretRef,
            &[VariableScope::Global],
        ),
        VariableDefinition::new(
            key("projects.favorite"),
            VariableType::ProjectId,
            &[VariableScope::Profile],
        ),
    ];
    let nodes = vec![
        node("start", NodeKind::Trigger),
        // Fine: a string into text, a project id into a project.
        map(
            probe_invoke("fine", "x"),
            "message",
            variable(shared("paths.root")),
        ),
        map(
            invoke("select", select_type(), crate::ActionParameters::new()),
            "project",
            variable(shared("projects.favorite")),
        ),
        // A project id cannot fill a whole number.
        map(
            probe_invoke("typed", "x"),
            "sleep_ms",
            variable(VariableRef::System(SystemVariable::ProjectCurrentId)),
        ),
        // Unknown variable, unknown node, a later node.
        map(
            probe_invoke("unknown", "x"),
            "message",
            variable(shared("paths.nothing")),
        ),
        map(
            probe_invoke("nonode", "x"),
            "message",
            template("{{ steps.ghost.output.value }}"),
        ),
        map(
            probe_invoke("later", "x"),
            "message",
            variable(VariableRef::Step(Reference {
                node: n("last"),
                output: "value".into(),
            })),
        ),
        // A credential name never becomes text.
        map(
            probe_invoke("secret", "x"),
            "message",
            template("token {{ shared.creds.token }}"),
        ),
        // The default must fit too.
        map(
            probe_invoke("default", "x"),
            "sleep_ms",
            Mapping::Variable {
                reference: shared("paths.root"),
                default: Some(VariableValue::Boolean(true)),
            },
        ),
        probe_invoke("last", "x"),
    ];
    let workflow = f.workflow(graph(
        nodes,
        chain(&[
            "start", "fine", "select", "typed", "unknown", "nonode", "later", "secret", "default",
            "last",
        ]),
    ));
    let graph = f.profile.workflow(workflow).unwrap().graph.clone().unwrap();
    let issues =
        graph.validate_with_variables(workflow, &f.profile, f.engine.registry(), &definitions);
    let on = |id: &str| -> Vec<&str> {
        issues
            .iter()
            .filter(|i| i.node.as_ref() == Some(&n(id)))
            .map(|i| i.message.as_str())
            .collect()
    };
    assert!(on("fine").is_empty(), "{issues:?}");
    assert!(on("select").is_empty(), "{issues:?}");
    assert_eq!(
        on("typed"),
        [
            "parameter `sleep_ms`: {{ system.project.current.id }} is a projectId value, the parameter expects a whole number"
        ]
    );
    assert_eq!(
        on("unknown"),
        ["{{ shared.paths.nothing }}: there is no shared variable `paths.nothing`"]
    );
    assert_eq!(
        on("nonode"),
        ["steps.ghost.output.value: `ghost` does not run before this node"]
    );
    assert_eq!(
        on("later"),
        ["steps.last.output.value: `last` does not run before this node"]
    );
    assert_eq!(
        on("secret"),
        ["{{ shared.creds.token }} is a secretRef value and cannot be part of text"]
    );
    assert_eq!(
        on("default"),
        [
            "parameter `sleep_ms`: {{ shared.paths.root }} is a string value, the parameter expects a whole number",
            "parameter `sleep_ms`: the default is a boolean value, the parameter expects a whole number"
        ]
    );
    // Without the definitions shared variables are left to the run; the
    // rest is the same.
    let without = f.issues(workflow);
    assert!(
        without.iter().all(|i| !i.message.contains("paths.nothing")),
        "{without:?}"
    );
    assert!(
        without
            .iter()
            .any(|i| i.message.contains("system.project.current.id"))
    );
}

#[test]
fn sensitive_shared_variables_only_reach_sensitive_outputs() {
    let mut f = Fixture::new();
    let mut definition = VariableDefinition::new(
        key("api.token"),
        VariableType::String,
        &[VariableScope::Global],
    );
    definition.sensitive = true;
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            map(
                node(
                    "out",
                    NodeKind::Output {
                        values: BTreeMap::new(),
                    },
                ),
                "token",
                variable(shared("api.token")),
            ),
        ],
        chain(&["start", "out"]),
    ));
    f.profile.workflows.last_mut().unwrap().outputs =
        vec![WorkflowOutput::new("token", ValueKind::Text)];
    let graph = f.profile.workflow(workflow).unwrap().graph.clone().unwrap();
    let issues = graph.validate_with_variables(
        workflow,
        &f.profile,
        f.engine.registry(),
        std::slice::from_ref(&definition),
    );
    assert_eq!(
        errors(&issues),
        ["`token` gets a sensitive value; mark the output sensitive so it stays out of history"]
    );
    // Not knowing the definition beforehand, the run refuses it instead,
    // without showing the value.
    let variables = MemoryVariables::new(f.profile.clone())
        .define(definition)
        .set(
            &key("api.token"),
            ScopeKey::Global,
            VariableValue::String("hunter2".into()),
        );
    let mut context = ActiveProjectContext::new(&f.profile);
    let report = f.run_resolved(workflow, &variables, &mut context);
    assert_eq!(report.status, RunStatus::Failed);
    let failed = report.node(&n("out")).unwrap();
    assert!(failed.output.is_empty());
    let error = failed.error.as_ref().unwrap();
    assert_eq!(error.code, "mapping");
    assert!(!error.message.contains("hunter2"));
}

// --- execution --------------------------------------------------------------

#[test]
fn inline_nodes_run_and_feed_later_nodes() {
    let mut f = Fixture::new();
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            probe_invoke("first", "inline"),
            map(
                probe_invoke("second", "unused"),
                "message",
                template("got {{ steps.first.output.value }}"),
            ),
        ],
        chain(&["start", "first", "second"]),
    ));
    assert!(f.issues(workflow).is_empty(), "{:?}", f.issues(workflow));
    let report = f.run(workflow);
    assert_eq!(report.status, RunStatus::Succeeded);
    assert_eq!(f.ran(), ["inline", "got inline"]);
    let first = report.node(&n("first")).unwrap();
    assert_eq!(first.action, None);
    assert_eq!(first.action_type, Some(probe_type()));
    assert_eq!(first.output["value"], "inline");
    // No instance was added to the Action Library.
    assert_eq!(f.profile.actions.len(), 0);
}

#[test]
fn inline_nodes_keep_retry_and_error_routing() {
    let mut f = Fixture::new();
    let mut flaky = invoke(
        "flaky",
        probe_type(),
        crate::ActionParameters::new()
            .with("message", ParameterValue::Text("flaky".into()))
            .with("fail_times", ParameterValue::Integer(2)),
    );
    if let NodeKind::Invoke { policy, .. } = &mut flaky.kind {
        policy.retry.attempts = 2;
    }
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            flaky,
            probe_invoke("broken", "fail"),
            probe_invoke("handled", "handled"),
        ],
        vec![
            edge("start", Port::Out, "flaky"),
            edge("flaky", Port::Out, "broken"),
            edge("broken", Port::Error, "handled"),
        ],
    ));
    assert!(f.issues(workflow).is_empty(), "{:?}", f.issues(workflow));
    let report = f.run(workflow);
    assert_eq!(report.node(&n("flaky")).unwrap().attempts, 3);
    assert_eq!(status(&report, "flaky"), NodeStatus::Succeeded);
    assert_eq!(status(&report, "broken"), NodeStatus::Failed);
    assert_eq!(status(&report, "handled"), NodeStatus::Succeeded);
    assert_eq!(report.status, RunStatus::Succeeded);
}

#[test]
fn shared_variables_fill_templates_and_typed_parameters() {
    let mut f = Fixture::new();
    let cms = f.project("CMS");
    let profile = f.profile.id;
    let variables = MemoryVariables::new(f.profile.clone())
        .define(VariableDefinition::new(
            key("paths.root"),
            VariableType::String,
            &[VariableScope::Global, VariableScope::Profile],
        ))
        .define(VariableDefinition::new(
            key("projects.favorite"),
            VariableType::ProjectId,
            &[VariableScope::Profile],
        ))
        .define(VariableDefinition::new(
            key("timing.wait"),
            VariableType::Integer,
            &[VariableScope::Global],
        ))
        .set(
            &key("paths.root"),
            ScopeKey::Global,
            VariableValue::String("/global".into()),
        )
        .set(
            &key("paths.root"),
            ScopeKey::Profile { profile },
            VariableValue::String("/work".into()),
        )
        .set(
            &key("projects.favorite"),
            ScopeKey::Profile { profile },
            VariableValue::ProjectId(cms),
        )
        .set(
            &key("timing.wait"),
            ScopeKey::Global,
            VariableValue::Integer(1),
        );
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            map(
                probe_invoke("cd", "unused"),
                "message",
                template("cd {{ shared.paths.root }} on {{ system.platform }}"),
            ),
            // A project id stays a project id: no detour through a key.
            map(
                invoke("select", select_type(), crate::ActionParameters::new()),
                "project",
                variable(shared("projects.favorite")),
            ),
            map(
                map(
                    probe_invoke("after", "unused"),
                    "message",
                    template("now {{ system.project.current.key }}"),
                ),
                "sleep_ms",
                variable(shared("timing.wait")),
            ),
            map(
                probe_invoke("fallback", "unused"),
                "message",
                Mapping::Variable {
                    reference: shared("paths.unset"),
                    default: Some(VariableValue::String("fallback".into())),
                },
            ),
        ],
        chain(&["start", "cd", "select", "after", "fallback"]),
    ));
    assert!(f.issues(workflow).is_empty(), "{:?}", f.issues(workflow));
    let mut context = ActiveProjectContext::new(&f.profile);
    let report = f.run_resolved(workflow, &variables, &mut context);
    assert_eq!(report.status, RunStatus::Succeeded, "{report:?}");
    // The profile value wins over the global one; the platform is the
    // resolver's.
    assert_eq!(f.ran(), ["cd /work on linux", "now CMS", "fallback"]);
    // The run's project scope decides about the active project; the run's
    // own context followed the selection.
    assert_eq!(context.project_id(), Some(cms));
}

#[test]
fn unresolved_references_fail_the_node_without_values() {
    let mut f = Fixture::new();
    let variables = MemoryVariables::new(f.profile.clone())
        .define(VariableDefinition::new(
            key("paths.root"),
            VariableType::String,
            &[VariableScope::Global],
        ))
        .define(VariableDefinition::new(
            key("names.secret"),
            VariableType::String,
            &[VariableScope::Global],
        ))
        .set(
            &key("names.secret"),
            ScopeKey::Global,
            VariableValue::String("classified".into()),
        );
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            map(
                probe_invoke("unset", "x"),
                "message",
                variable(shared("paths.root")),
            ),
            map(
                probe_invoke("unknown", "x"),
                "message",
                template("{{ shared.paths.other }}"),
            ),
            map(
                probe_invoke("typed", "x"),
                "sleep_ms",
                variable(shared("names.secret")),
            ),
            probe_invoke("never", "never"),
        ],
        vec![
            edge("start", Port::Out, "unset"),
            edge("unset", Port::Error, "unknown"),
            edge("unknown", Port::Error, "typed"),
            edge("typed", Port::Out, "never"),
        ],
    ));
    let mut context = ActiveProjectContext::new(&f.profile);
    let report = f.run_resolved(workflow, &variables, &mut context);
    let error = |id: &str| report.node(&n(id)).unwrap().error.clone().unwrap();
    assert_eq!(error("unset").code, "reference-missing");
    assert_eq!(
        error("unset").message,
        "parameter `message`: {{ shared.paths.root }} has no value (no value is set)"
    );
    assert_eq!(error("unknown").code, "reference-missing");
    assert_eq!(
        error("unknown").message,
        "parameter `message`: {{ shared.paths.other }} has no value (no such variable)"
    );
    assert_eq!(error("typed").code, "reference-type");
    assert_eq!(
        error("typed").message,
        "parameter `sleep_ms`: {{ shared.names.secret }} is a string value, the parameter expects an integer"
    );
    assert_eq!(status(&report, "never"), NodeStatus::Skipped);
    assert_eq!(f.ran(), Vec::<String>::new());

    // Without a resolver the same references are unavailable.
    let report = f.run(workflow);
    assert_eq!(
        report
            .node(&n("unset"))
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .code,
        "reference-unavailable"
    );
}

#[test]
fn typed_node_outputs_and_variables_feed_typed_parameters() {
    let mut f = Fixture::new();
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            map(
                node(
                    "set",
                    NodeKind::SetVariable {
                        values: BTreeMap::new(),
                    },
                ),
                "wait",
                template("{{ steps.start.output.wait }}"),
            ),
            map(
                map(
                    probe_invoke("use", "unused"),
                    "sleep_ms",
                    Mapping::Output {
                        reference: Reference {
                            node: n("vars"),
                            output: "wait".into(),
                        },
                        default: None,
                    },
                ),
                "message",
                variable(VariableRef::Step(Reference {
                    node: n("start"),
                    output: "wait".into(),
                })),
            ),
        ],
        chain(&["start", "set", "use"]),
    ));
    {
        let definition = f.profile.workflows.last_mut().unwrap();
        let mut input = WorkflowInput::new("wait", ValueKind::Integer);
        input.default = Some(" 3 ".into());
        definition.inputs = vec![input];
        definition.variables = vec![WorkflowVariable::new("wait", ValueKind::Integer)];
    }
    assert!(f.issues(workflow).is_empty(), "{:?}", f.issues(workflow));
    let report = f.run(workflow);
    assert_eq!(report.status, RunStatus::Succeeded, "{report:?}");
    // The variable holds the converted integer; the input text is passed as
    // it is to a text parameter.
    assert_eq!(report.node(&n("set")).unwrap().output["wait"], "3");
    assert_eq!(f.ran(), [" 3 "]);
}

#[test]
fn concurrent_runs_resolve_against_their_own_project() {
    let mut f = Fixture::new();
    let shop = f.project("SHOP");
    let cms = f.project("CMS");
    let variables = MemoryVariables::new(f.profile.clone());
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            map(
                invoke(
                    "slow",
                    probe_type(),
                    crate::ActionParameters::new().with("sleep_ms", ParameterValue::Integer(50)),
                ),
                "message",
                template("{{ system.project.current.key }} in {{ workflow.execution.id }}"),
            ),
            map(
                probe_invoke("id", "unused"),
                "message",
                variable(VariableRef::System(SystemVariable::ProjectCurrentName)),
            ),
        ],
        chain(&["start", "slow", "id"]),
    ));
    let runs: Vec<(crate::ProjectId, GraphReport)> = std::thread::scope(|scope| {
        let handles: Vec<_> = [shop, cms, shop, cms]
            .into_iter()
            .map(|project| {
                let (f, variables) = (&f, &variables);
                scope.spawn(move || {
                    let mut context = ActiveProjectContext::restore(&f.profile, Some(project));
                    (project, f.run_resolved(workflow, variables, &mut context))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut executions = std::collections::BTreeSet::new();
    for (project, report) in runs {
        assert_eq!(report.status, RunStatus::Succeeded);
        let key = f.profile.project(project).unwrap().key.to_string();
        let slow = &report.node(&n("slow")).unwrap().output["value"];
        let (seen, execution) = slow.split_once(" in ").unwrap();
        assert_eq!(seen, key);
        executions.insert(execution.to_owned());
        assert_eq!(report.node(&n("id")).unwrap().output["value"], key);
    }
    // Every run has its own execution id.
    assert_eq!(executions.len(), 4);
}

#[test]
fn a_selection_inside_a_run_changes_its_references_only_after_it() {
    let mut f = Fixture::new();
    let cms = f.project("CMS");
    let variables = MemoryVariables::new(f.profile.clone());
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            node("fork", NodeKind::Parallel),
            invoke(
                "select",
                select_type(),
                crate::ActionParameters::new().with("project", ParameterValue::Project(cms)),
            ),
            map(
                probe_invoke("after", "x"),
                "message",
                template("{{ system.project.current.key }}"),
            ),
            map(
                probe_invoke("beside", "x"),
                "message",
                template("{{ system.project.current.key }}"),
            ),
        ],
        vec![
            edge("start", Port::Out, "fork"),
            edge("fork", Port::Out, "select"),
            edge("select", Port::Out, "after"),
            edge("fork", Port::Out, "beside"),
        ],
    ));
    let mut context = ActiveProjectContext::new(&f.profile);
    let report = f.run_resolved(workflow, &variables, &mut context);
    assert_eq!(report.status, RunStatus::Succeeded, "{report:?}");
    assert_eq!(report.node(&n("after")).unwrap().output["value"], "CMS");
    // The parallel branch keeps the run's initial project.
    assert_eq!(report.node(&n("beside")).unwrap().output["value"], "SHOP");
}

#[test]
fn inline_project_selection_counts_for_the_project_scope() {
    let mut f = Fixture::new();
    let workflow = f.workflow(graph(
        vec![
            node("start", NodeKind::Trigger),
            invoke("select", select_type(), crate::ActionParameters::new()),
        ],
        chain(&["start", "select"]),
    ));
    let definition = f.profile.workflow(workflow).unwrap();
    assert!(definition.selects_projects(&f.profile, f.engine.registry()));
}
