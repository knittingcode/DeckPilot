//! References to shared, system and workflow variables in node mappings
//! (whole-value `variable` mappings and templates), checked before the run
//! and resolved in each node's own context, and the stored graphs that must
//! keep loading and running unchanged.

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

thread_local! {
    /// Instances created by [`step`] for the next [`adopt`].
    static PENDING: std::cell::RefCell<Vec<ActionInstance>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// An action node running a new saved instance of `action_type`, which
/// [`adopt`] adds to the profile.
fn step(id: &str, action_type: crate::ActionTypeId, parameters: crate::ActionParameters) -> Node {
    let instance = ActionInstance {
        id: crate::ActionInstanceId::new(),
        name: id.into(),
        action_type,
        parameters,
        description: String::new(),
        icon: None,
        label: None,
    };
    let node = action(id, instance.id);
    PENDING.with(|pending| pending.borrow_mut().push(instance));
    node
}

fn probe_step(id: &str, message: &str) -> Node {
    step(
        id,
        probe_type(),
        crate::ActionParameters::new().with("message", ParameterValue::Text(message.into())),
    )
}

/// Adds the instances of the graph's steps, then the workflow.
fn adopt(f: &mut Fixture, graph: WorkflowGraph) -> WorkflowId {
    PENDING.with(|pending| f.profile.actions.append(&mut pending.borrow_mut()));
    f.workflow(graph)
}

fn map(mut node: Node, parameter: &str, mapping: Mapping) -> Node {
    match &mut node.kind {
        NodeKind::Action { mappings, .. } => {
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

// --- stored graphs -----------------------------------------------------------

/// A graph as 0.6.0 stores it: saved instances, `steps.`
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
fn stored_graphs_load_run_and_store_unchanged() {
    let mut f = Fixture::new();
    let first = f.probe("alpha", &[]);
    let second = f.probe("unused", &[]);
    let stored = legacy_json(first, second);
    let graph: WorkflowGraph = serde_json::from_str(&stored).unwrap();
    assert_eq!(graph.version, GRAPH_VERSION);
    // Stored again, it is the same document.
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
fn the_stored_shapes_match_the_contract() {
    let node = map(
        map(
            step("open", select_type(), crate::ActionParameters::new()),
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
    assert_eq!(json["type"], "action");
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
            probe_step("fine", "x"),
            "message",
            variable(shared("paths.root")),
        ),
        map(
            step("select", select_type(), crate::ActionParameters::new()),
            "project",
            variable(shared("projects.favorite")),
        ),
        // A project id cannot fill a whole number.
        map(
            probe_step("typed", "x"),
            "sleep_ms",
            variable(VariableRef::System(SystemVariable::ProjectCurrentId)),
        ),
        // Unknown variable, unknown node, a later node.
        map(
            probe_step("unknown", "x"),
            "message",
            variable(shared("paths.nothing")),
        ),
        map(
            probe_step("nonode", "x"),
            "message",
            template("{{ steps.ghost.output.value }}"),
        ),
        map(
            probe_step("later", "x"),
            "message",
            variable(VariableRef::Step(Reference {
                node: n("last"),
                output: "value".into(),
            })),
        ),
        // A credential name never becomes text.
        map(
            probe_step("secret", "x"),
            "message",
            template("token {{ shared.creds.token }}"),
        ),
        // The default must fit too.
        map(
            probe_step("default", "x"),
            "sleep_ms",
            Mapping::Variable {
                reference: shared("paths.root"),
                default: Some(VariableValue::Boolean(true)),
            },
        ),
        probe_step("last", "x"),
    ];
    let workflow = adopt(
        &mut f,
        graph(
            nodes,
            chain(&[
                "start", "fine", "select", "typed", "unknown", "nonode", "later", "secret",
                "default", "last",
            ]),
        ),
    );
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
    let workflow = adopt(
        &mut f,
        graph(
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
        ),
    );
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
    let workflow = adopt(
        &mut f,
        graph(
            vec![
                node("start", NodeKind::Trigger),
                map(
                    probe_step("cd", "unused"),
                    "message",
                    template("cd {{ shared.paths.root }} on {{ system.platform }}"),
                ),
                // A project id stays a project id: no detour through a key.
                map(
                    step("select", select_type(), crate::ActionParameters::new()),
                    "project",
                    variable(shared("projects.favorite")),
                ),
                map(
                    map(
                        probe_step("after", "unused"),
                        "message",
                        template("now {{ system.project.current.key }}"),
                    ),
                    "sleep_ms",
                    variable(shared("timing.wait")),
                ),
                map(
                    probe_step("fallback", "unused"),
                    "message",
                    Mapping::Variable {
                        reference: shared("paths.unset"),
                        default: Some(VariableValue::String("fallback".into())),
                    },
                ),
            ],
            chain(&["start", "cd", "select", "after", "fallback"]),
        ),
    );
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
    let workflow = adopt(
        &mut f,
        graph(
            vec![
                node("start", NodeKind::Trigger),
                map(
                    probe_step("unset", "x"),
                    "message",
                    variable(shared("paths.root")),
                ),
                map(
                    probe_step("unknown", "x"),
                    "message",
                    template("{{ shared.paths.other }}"),
                ),
                map(
                    probe_step("typed", "x"),
                    "sleep_ms",
                    variable(shared("names.secret")),
                ),
                probe_step("never", "never"),
            ],
            vec![
                edge("start", Port::Out, "unset"),
                edge("unset", Port::Error, "unknown"),
                edge("unknown", Port::Error, "typed"),
                edge("typed", Port::Out, "never"),
            ],
        ),
    );
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
    let workflow = adopt(
        &mut f,
        graph(
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
                        probe_step("use", "unused"),
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
        ),
    );
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
    let workflow = adopt(
        &mut f,
        graph(
            vec![
                node("start", NodeKind::Trigger),
                map(
                    step(
                        "slow",
                        probe_type(),
                        crate::ActionParameters::new()
                            .with("sleep_ms", ParameterValue::Integer(50)),
                    ),
                    "message",
                    template("{{ system.project.current.key }} in {{ workflow.execution.id }}"),
                ),
                map(
                    probe_step("id", "unused"),
                    "message",
                    variable(VariableRef::System(SystemVariable::ProjectCurrentName)),
                ),
            ],
            chain(&["start", "slow", "id"]),
        ),
    );
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
    let workflow = adopt(
        &mut f,
        graph(
            vec![
                node("start", NodeKind::Trigger),
                node("fork", NodeKind::Parallel),
                step(
                    "select",
                    select_type(),
                    crate::ActionParameters::new().with("project", ParameterValue::Project(cms)),
                ),
                map(
                    probe_step("after", "x"),
                    "message",
                    template("{{ system.project.current.key }}"),
                ),
                map(
                    probe_step("beside", "x"),
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
        ),
    );
    let mut context = ActiveProjectContext::new(&f.profile);
    let report = f.run_resolved(workflow, &variables, &mut context);
    assert_eq!(report.status, RunStatus::Succeeded, "{report:?}");
    assert_eq!(report.node(&n("after")).unwrap().output["value"], "CMS");
    // The parallel branch keeps the run's initial project.
    assert_eq!(report.node(&n("beside")).unwrap().output["value"], "SHOP");
}

#[test]
fn references_filling_required_parameters_are_still_checked() {
    // `test.echo` requires `message`; a mapping supplies it.
    let registry = crate::testing::registry();
    let mut f = Fixture::new();
    let echo = crate::ActionTypeId::new("test.echo").unwrap();
    let nodes = vec![
        node("start", NodeKind::Trigger),
        map(
            step("legacy", echo.clone(), crate::ActionParameters::new()),
            "message",
            template("{{ steps.start.output.text }}"),
        ),
        map(
            step("secret", echo, crate::ActionParameters::new()),
            "message",
            variable(shared("creds.token")),
        ),
    ];
    let workflow = adopt(&mut f, graph(nodes, chain(&["start", "legacy", "secret"])));
    f.profile.workflows.last_mut().unwrap().inputs =
        vec![WorkflowInput::new("text", ValueKind::Text)];
    let graph = f.profile.workflow(workflow).unwrap().graph.clone().unwrap();
    let definitions = [VariableDefinition::new(
        key("creds.token"),
        VariableType::SecretRef,
        &[VariableScope::Global],
    )];
    let issues = graph.validate_with_variables(workflow, &f.profile, &registry, &definitions);
    assert_eq!(
        errors(&issues),
        [
            "parameter `message`: {{ shared.creds.token }} is a secretRef value, the parameter expects text"
        ]
    );
}
