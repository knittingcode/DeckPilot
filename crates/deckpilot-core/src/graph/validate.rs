//! Structural and semantic checks of a workflow graph. They run before a
//! graph is saved and again before every run, so a graph that became
//! invalid (a module was disabled, an action deleted) is never executed.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::mapping::{convert, is_value_key};
use super::{
    ErrorPolicy, GRAPH_VERSION, MAX_DELAY_MS, MAX_LOOP_ITERATIONS, MAX_NESTING, MAX_NODES,
    MAX_RETRIES, MAX_TIMEOUT_MS, MAX_WORKFLOW_VALUES, Mapping, NodeId, NodeKind, Port, Predicate,
    Reference, VARIABLES_NODE, ValueKind, WorkflowGraph, WorkflowInput, WorkflowOutput,
    WorkflowVariable,
};
use crate::variables::{VariableDefinition, VariableRef, VariableType};
use crate::{ActionRegistry, ParameterKind, Profile, Workflow, WorkflowId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueSeverity {
    /// Prevents saving and running.
    Error,
    /// Worth fixing; does not prevent running.
    Warning,
}

/// A problem, attached to a node where there is one, so that the editor can
/// show it on the node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphIssue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
    pub severity: IssueSeverity,
    pub message: String,
}

impl GraphIssue {
    fn error(node: Option<&NodeId>, message: impl Into<String>) -> Self {
        Self {
            node: node.cloned(),
            severity: IssueSeverity::Error,
            message: message.into(),
        }
    }

    fn warning(node: Option<&NodeId>, message: impl Into<String>) -> Self {
        Self {
            node: node.cloned(),
            severity: IssueSeverity::Warning,
            message: message.into(),
        }
    }
}

struct Checker<'a> {
    graph: &'a WorkflowGraph,
    profile: &'a Profile,
    registry: &'a ActionRegistry,
    issues: Vec<GraphIssue>,
    /// Ancestors of each node (nodes on some path from the trigger to it).
    ancestors: BTreeMap<&'a NodeId, BTreeSet<&'a NodeId>>,
    /// The owning workflow's declared inputs and outputs.
    inputs: &'a [WorkflowInput],
    outputs: &'a [WorkflowOutput],
    variables: &'a [WorkflowVariable],
    /// Shared variable definitions, when the caller knows them; otherwise
    /// `shared.` references are checked when they are resolved.
    shared: Option<&'a [VariableDefinition]>,
}

/// What a reference is known to hold.
#[derive(Default)]
struct Found {
    sensitive: bool,
    /// `None`: not known before the run (node outputs are text).
    value_type: Option<VariableType>,
}

/// What a referenced node provides under an output key.
enum Provided {
    /// Cannot be told (the source itself has a problem, reported there).
    Unknown,
    No,
    Yes {
        sensitive: bool,
    },
}

impl WorkflowGraph {
    /// Every problem of the graph of workflow `owner` in `profile`, with the
    /// inputs and outputs `owner` declares there. The graph may run if no
    /// issue is an [`IssueSeverity::Error`].
    pub fn validate(
        &self,
        owner: WorkflowId,
        profile: &Profile,
        registry: &ActionRegistry,
    ) -> Vec<GraphIssue> {
        let (inputs, outputs, variables) = profile
            .workflow(owner)
            .map(|w| {
                (
                    w.inputs.as_slice(),
                    w.outputs.as_slice(),
                    w.variables.as_slice(),
                )
            })
            .unwrap_or_default();
        self.check(owner, (inputs, outputs, variables), profile, registry, None)
    }

    /// [`WorkflowGraph::validate`] that also checks `shared.` references
    /// against the shared variable definitions: unknown variables, types
    /// that cannot fill their parameter, and sensitive values flowing into
    /// values that are not marked sensitive.
    pub fn validate_with_variables(
        &self,
        owner: WorkflowId,
        profile: &Profile,
        registry: &ActionRegistry,
        shared: &[VariableDefinition],
    ) -> Vec<GraphIssue> {
        let (inputs, outputs, variables) = profile
            .workflow(owner)
            .map(|w| {
                (
                    w.inputs.as_slice(),
                    w.outputs.as_slice(),
                    w.variables.as_slice(),
                )
            })
            .unwrap_or_default();
        self.check(
            owner,
            (inputs, outputs, variables),
            profile,
            registry,
            Some(shared),
        )
    }

    /// Every problem of this graph as the graph of `workflow`, which need
    /// not be saved: its own inputs and outputs are used.
    pub fn validate_as(
        &self,
        workflow: &Workflow,
        profile: &Profile,
        registry: &ActionRegistry,
    ) -> Vec<GraphIssue> {
        self.check(
            workflow.id,
            (&workflow.inputs, &workflow.outputs, &workflow.variables),
            profile,
            registry,
            None,
        )
    }

    fn check(
        &self,
        owner: WorkflowId,
        (inputs, outputs, variables): (&[WorkflowInput], &[WorkflowOutput], &[WorkflowVariable]),
        profile: &Profile,
        registry: &ActionRegistry,
        shared: Option<&[VariableDefinition]>,
    ) -> Vec<GraphIssue> {
        let mut checker = Checker {
            graph: self,
            profile,
            registry,
            issues: Vec::new(),
            ancestors: BTreeMap::new(),
            inputs,
            outputs,
            variables,
            shared,
        };
        checker.declarations();
        checker.variable_declarations();
        // Semantic checks need an acyclic graph whose connections all refer
        // to existing nodes; other structural problems do not hide them.
        if checker.structure() {
            checker.compute_ancestors();
            checker.nodes(owner);
            checker.parallel_writes();
        }
        checker.issues
    }

    /// Whether the graph may run.
    pub fn is_runnable(issues: &[GraphIssue]) -> bool {
        issues.iter().all(|i| i.severity != IssueSeverity::Error)
    }
}

impl<'a> Checker<'a> {
    /// Structural checks; `true` if semantic checks can run.
    fn structure(&mut self) -> bool {
        let graph = self.graph;
        if graph.version != GRAPH_VERSION {
            self.issues.push(GraphIssue::error(
                None,
                format!("graph format {} is not supported", graph.version),
            ));
            return false;
        }
        let mut analysable = true;
        if graph.nodes.len() > MAX_NODES {
            self.issues.push(GraphIssue::error(
                None,
                format!("a workflow can have at most {MAX_NODES} nodes"),
            ));
        }
        let unique = self.identities();
        let mut edges = BTreeSet::new();
        let mut per_port: BTreeMap<(&NodeId, Port), usize> = BTreeMap::new();
        for edge in &graph.edges {
            let (Some(from), Some(to)) = (graph.node(&edge.from), graph.node(&edge.to)) else {
                analysable = false;
                self.issues.push(GraphIssue::error(
                    graph.node(&edge.from).map(|n| &n.id),
                    format!(
                        "a connection refers to a missing node ({} → {})",
                        edge.from, edge.to
                    ),
                ));
                continue;
            };
            if edge.from == edge.to {
                self.issues.push(GraphIssue::error(
                    Some(&from.id),
                    "a node cannot connect to itself",
                ));
            }
            if !from.kind.ports().contains(&edge.port) {
                self.issues.push(GraphIssue::error(
                    Some(&from.id),
                    format!("a {} node has no {:?} output", from.kind.name(), edge.port),
                ));
            }
            if matches!(to.kind, NodeKind::Trigger) {
                self.issues.push(GraphIssue::error(
                    Some(&to.id),
                    "nothing can connect into the start node",
                ));
            }
            if !edges.insert((&edge.from, edge.port, &edge.to)) {
                self.issues.push(GraphIssue::error(
                    Some(&from.id),
                    "the same connection exists twice",
                ));
            }
            *per_port.entry((&edge.from, edge.port)).or_default() += 1;
        }
        for ((id, port), count) in per_port {
            let parallel = matches!(graph.node(id).map(|n| &n.kind), Some(NodeKind::Parallel));
            if count > 1 && !parallel {
                self.issues.push(GraphIssue::error(
                    Some(id),
                    format!(
                        "the {port:?} output connects to {count} nodes; branch with a Parallel or Condition node"
                    ),
                ));
            }
        }
        self.degrees();
        if graph.topological_order().is_none() {
            analysable = false;
            self.issues.push(GraphIssue::error(
                None,
                "the workflow contains a cycle; use a Loop node to repeat steps",
            ));
        }
        analysable && unique
    }

    /// Unique node ids and exactly one start node; `true` if ids are unique.
    fn identities(&mut self) -> bool {
        let graph = self.graph;
        let mut seen = BTreeSet::new();
        for node in &graph.nodes {
            if !seen.insert(&node.id) {
                self.issues.push(GraphIssue::error(
                    Some(&node.id),
                    format!("the node id `{}` is used twice", node.id),
                ));
            }
        }
        let triggers: Vec<_> = graph
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Trigger))
            .collect();
        match triggers.as_slice() {
            [] => self
                .issues
                .push(GraphIssue::error(None, "the workflow needs a start node")),
            [_] => {}
            [_, extra @ ..] => {
                for node in extra {
                    self.issues.push(GraphIssue::error(
                        Some(&node.id),
                        "a workflow has exactly one start node",
                    ));
                }
            }
        }

        seen.len() == graph.nodes.len()
    }

    /// Incoming and outgoing connection counts per node kind.
    fn degrees(&mut self) {
        let graph = self.graph;
        for node in &graph.nodes {
            let incoming = graph.incoming(&node.id).count();
            match node.kind {
                NodeKind::Join if incoming < 2 => self.issues.push(GraphIssue::warning(
                    Some(&node.id),
                    "a Join node waits for two or more branches",
                )),
                NodeKind::Trigger | NodeKind::Join => {}
                _ if incoming == 0 => self.issues.push(GraphIssue::error(
                    Some(&node.id),
                    "this node is not connected to the workflow",
                )),
                _ if incoming > 1 => self.issues.push(GraphIssue::error(
                    Some(&node.id),
                    "only a Join node can have several incoming connections",
                )),
                _ => {}
            }
            if matches!(node.kind, NodeKind::Parallel) && graph.outgoing(&node.id).count() < 2 {
                self.issues.push(GraphIssue::warning(
                    Some(&node.id),
                    "a Parallel node needs at least two branches",
                ));
            }
        }
    }

    fn compute_ancestors(&mut self) {
        let graph = self.graph;
        let Some(order) = graph.topological_order() else {
            return;
        };
        for id in order {
            let mut set = BTreeSet::new();
            for edge in graph.incoming(id) {
                set.insert(&edge.from);
                if let Some(parent) = self.ancestors.get(&edge.from) {
                    set.extend(parent.iter().copied());
                }
            }
            self.ancestors.insert(id, set);
        }
    }

    fn is_ancestor(&self, ancestor: &NodeId, of: &NodeId) -> bool {
        self.ancestors
            .get(of)
            .is_some_and(|set| set.contains(ancestor))
    }

    /// Checks that `reference` can be resolved at `node`; `true` if the
    /// value it refers to is sensitive.
    fn check_reference(&mut self, node: &NodeId, reference: &Reference) -> bool {
        if self.is_variable(reference) {
            // A variable: readable anywhere, with the value on the path.
            if let Some(variable) = self.variables.iter().find(|v| v.key == reference.output) {
                return variable.sensitive;
            }
            self.issues.push(GraphIssue::error(
                Some(node),
                format!(
                    "{reference}: this workflow has no variable `{}`",
                    reference.output
                ),
            ));
            return false;
        }
        if !self.is_ancestor(&reference.node, node) {
            self.issues.push(GraphIssue::error(
                Some(node),
                format!(
                    "{reference}: `{}` does not run before this node",
                    reference.node
                ),
            ));
            return false;
        }
        match self.provided(reference) {
            Provided::Yes { sensitive } => sensitive,
            Provided::Unknown => false,
            Provided::No => {
                self.issues.push(GraphIssue::error(
                    Some(node),
                    format!(
                        "{reference}: `{}` provides no output `{}`",
                        reference.node, reference.output
                    ),
                ));
                false
            }
        }
    }

    /// What the node `reference` points to provides under its key: an
    /// action its declared outputs, the start node the workflow's inputs,
    /// a sub-workflow the called workflow's outputs.
    fn provided(&self, reference: &Reference) -> Provided {
        let Some(source) = self.graph.node(&reference.node) else {
            return Provided::No;
        };
        let key = reference.output.as_str();
        let found = |sensitive: Option<bool>| match sensitive {
            Some(sensitive) => Provided::Yes { sensitive },
            None => Provided::No,
        };
        match &source.kind {
            NodeKind::Action { action, .. } => {
                let Some(definition) = self
                    .profile
                    .action(*action)
                    .and_then(|instance| self.registry.definition(&instance.action_type))
                else {
                    return Provided::Unknown;
                };
                found(
                    definition
                        .node
                        .outputs
                        .iter()
                        .find(|o| o.key == key)
                        .map(|o| o.sensitive),
                )
            }
            NodeKind::Trigger => found(
                self.inputs
                    .iter()
                    .find(|i| i.key == key)
                    .map(|i| i.sensitive),
            ),
            NodeKind::Call { workflow, .. } => match self.profile.workflow(*workflow) {
                Some(callee) => found(
                    callee
                        .outputs
                        .iter()
                        .find(|o| o.key == key)
                        .map(|o| o.sensitive),
                ),
                None => Provided::Unknown,
            },
            _ => Provided::No,
        }
    }

    /// Checks the references of a mapping at `node`; `true` if the value
    /// can contain a sensitive one. A template renders every reference as
    /// text, so values that never become text (credential names,
    /// resources) cannot be part of one.
    fn check_mapping(&mut self, node: &NodeId, mapping: &Mapping) -> bool {
        let references = match mapping.references() {
            Ok(references) => references,
            Err(error) => {
                self.issues
                    .push(GraphIssue::error(Some(node), error.to_string()));
                return false;
            }
        };
        let mut sensitive = false;
        for reference in &references {
            let found = self.check_variable(node, reference);
            sensitive |= found.sensitive;
            if matches!(mapping, Mapping::Template { .. })
                && let Some(value_type) = found.value_type
                && !renders_as_text(value_type)
            {
                self.issues.push(GraphIssue::error(
                    Some(node),
                    format!(
                        "{{{{ {reference} }}}} is a {} value and cannot be part of text",
                        value_type.name()
                    ),
                ));
            }
        }
        sensitive
    }

    /// Checks one reference of any root at `node`.
    fn check_variable(&mut self, node: &NodeId, reference: &VariableRef) -> Found {
        match reference {
            VariableRef::Step(step) => Found {
                sensitive: self.check_reference(node, step),
                value_type: None,
            },
            VariableRef::System(variable) => Found {
                sensitive: false,
                value_type: Some(variable.value_type()),
            },
            // Declared by a module: checked when it is resolved, against
            // the module registry of the agent that runs it.
            VariableRef::Module(_) => Found::default(),
            // Without the definitions the variable is checked when it is
            // resolved at run time.
            VariableRef::Shared(key) => match self.shared {
                None => Found::default(),
                Some(definitions) => {
                    if let Some(definition) = definitions.iter().find(|d| &d.key == key) {
                        Found {
                            sensitive: definition.sensitive,
                            value_type: Some(definition.value_type),
                        }
                    } else {
                        self.issues.push(GraphIssue::error(
                            Some(node),
                            format!("{{{{ {reference} }}}}: there is no shared variable `{key}`"),
                        ));
                        Found::default()
                    }
                }
            },
        }
    }

    /// Checks a mapping that fills parameter `key` of `kind`; `true` if the
    /// value can contain a sensitive one.
    ///
    /// A whole-value reference whose type is known (system variables,
    /// shared variables with known definitions) must fit the parameter, and
    /// so must its default. Node outputs are text converted at run time;
    /// a declared output kind that can never fit is a warning, so graphs
    /// that were accepted before keep running.
    fn check_typed_mapping(
        &mut self,
        node: &NodeId,
        key: &str,
        mapping: &Mapping,
        kind: &ParameterKind,
    ) -> bool {
        let sensitive = self.check_mapping(node, mapping);
        let step = match mapping {
            Mapping::Output { reference, .. }
            | Mapping::Variable {
                reference: VariableRef::Step(reference),
                ..
            } => Some(reference),
            Mapping::Variable { reference, default } => {
                let found = self.peek_type(reference);
                if let Some(value_type) = found
                    && !value_type.assignable_to(kind)
                {
                    self.issues.push(GraphIssue::error(
                        Some(node),
                        format!(
                            "parameter `{key}`: {{{{ {reference} }}}} is a {} value, the parameter expects {}",
                            value_type.name(),
                            expects(kind)
                        ),
                    ));
                }
                if let Some(default) = default
                    && default.to_parameter(kind).is_none()
                {
                    self.issues.push(GraphIssue::error(
                        Some(node),
                        format!(
                            "parameter `{key}`: the default is a {} value, the parameter expects {}",
                            default.value_type().name(),
                            expects(kind)
                        ),
                    ));
                }
                None
            }
            Mapping::Template { .. } => None,
        };
        if let Some(reference) = step {
            if matches!(kind, ParameterKind::Resource { .. }) {
                self.issues.push(GraphIssue::warning(
                    Some(node),
                    format!(
                        "parameter `{key}` is a resource; a node output cannot fill it, so the node fails when it runs"
                    ),
                ));
            } else if let Some(output) = self.output_kind(reference)
                && !output_fits(output, kind)
            {
                self.issues.push(GraphIssue::warning(
                    Some(node),
                    format!(
                        "parameter `{key}`: {reference} is {}, the parameter expects {}",
                        output_name(output),
                        expects(kind)
                    ),
                ));
            }
        }
        sensitive
    }

    /// The known type of a non-`steps` reference, without reporting
    /// anything (`check_mapping` did).
    fn peek_type(&self, reference: &VariableRef) -> Option<VariableType> {
        match reference {
            VariableRef::Step(_) | VariableRef::Module(_) => None,
            VariableRef::System(variable) => Some(variable.value_type()),
            VariableRef::Shared(key) => self
                .shared?
                .iter()
                .find(|d| &d.key == key)
                .map(|d| d.value_type),
        }
    }

    /// The declared kind of the output `reference` points to, if known.
    fn output_kind(&self, reference: &Reference) -> Option<ValueKind> {
        if self.is_variable(reference) {
            return self
                .variables
                .iter()
                .find(|v| v.key == reference.output)
                .map(|v| v.kind);
        }
        let source = self.graph.node(&reference.node)?;
        let key = reference.output.as_str();
        match &source.kind {
            NodeKind::Action { action, .. } => {
                let instance = self.profile.action(*action)?;
                self.declared_output(&instance.action_type, key)
            }
            NodeKind::Trigger => self.inputs.iter().find(|i| i.key == key).map(|i| i.kind),
            NodeKind::Call { workflow, .. } => self
                .profile
                .workflow(*workflow)?
                .outputs
                .iter()
                .find(|o| o.key == key)
                .map(|o| o.kind),
            _ => None,
        }
    }

    fn declared_output(&self, action_type: &crate::ActionTypeId, key: &str) -> Option<ValueKind> {
        self.registry
            .definition(action_type)?
            .node
            .outputs
            .iter()
            .find(|o| o.key == key)
            .map(|o| o.kind)
    }

    /// Whether `reference` reads a variable (`steps.vars.output.<key>` in a
    /// workflow that declares variables).
    fn is_variable(&self, reference: &Reference) -> bool {
        !self.variables.is_empty() && reference.node.as_str() == VARIABLES_NODE
    }

    /// The declared variables: valid, unique keys, initial values of their
    /// kind, and the `vars` node id reserved.
    fn variable_declarations(&mut self) {
        if self.variables.is_empty() {
            return;
        }
        if self.variables.len() > MAX_WORKFLOW_VALUES {
            self.issues.push(GraphIssue::error(
                None,
                format!("a workflow can have at most {MAX_WORKFLOW_VALUES} variables"),
            ));
        }
        let mut seen = BTreeSet::new();
        for variable in self.variables {
            let key = &variable.key;
            if !is_value_key(key) {
                self.issues.push(GraphIssue::error(
                    None,
                    format!(
                        "`{key}` is not a valid variable key: use 1–64 letters, digits and `_`"
                    ),
                ));
            } else if !seen.insert(key) {
                self.issues.push(GraphIssue::error(
                    None,
                    format!("the variable `{key}` is declared twice"),
                ));
            }
            if let Some(initial) = &variable.initial
                && let Err(error) = convert(variable.kind, initial, self.profile)
            {
                self.issues.push(GraphIssue::error(
                    None,
                    format!("the initial value of the variable `{key}` {error}"),
                ));
            }
            let set = self.graph.nodes.iter().any(
                |n| matches!(&n.kind, NodeKind::SetVariable { values } if values.contains_key(key)),
            );
            if !set && variable.initial.is_none() {
                self.issues.push(GraphIssue::warning(
                    None,
                    format!("the variable `{key}` has no initial value and no node sets it"),
                ));
            }
        }
        for node in &self.graph.nodes {
            if node.id.as_str() == VARIABLES_NODE {
                self.issues.push(GraphIssue::error(
                    Some(&node.id),
                    format!(
                        "the node id `{VARIABLES_NODE}` is reserved for variables; rename this node"
                    ),
                ));
            }
        }
    }

    /// A Set variable node sets declared variables; sensitive values only
    /// sensitive ones.
    fn set_variable_node(&mut self, id: &NodeId, values: &BTreeMap<String, Mapping>) {
        if values.is_empty() {
            self.issues
                .push(GraphIssue::warning(Some(id), "this node sets no variables"));
        }
        for (key, mapping) in values {
            let sensitive = self.check_mapping(id, mapping);
            match self.variables.iter().find(|v| &v.key == key) {
                None => self.issues.push(GraphIssue::error(
                    Some(id),
                    format!("`{key}` is not a variable of this workflow; declare it first"),
                )),
                Some(variable) if sensitive && !variable.sensitive => {
                    self.issues.push(GraphIssue::error(
                        Some(id),
                        format!(
                            "`{key}` gets a sensitive value; mark the variable sensitive so it stays out of history"
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
    }

    /// Set variable nodes in different branches of a Parallel node that set
    /// the same variable: the Join after them fails at run time unless they
    /// set the same value.
    fn parallel_writes(&mut self) {
        let graph = self.graph;
        let writers: Vec<(&NodeId, &String)> = graph
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::SetVariable { values } => Some(values.keys().map(move |k| (&n.id, k))),
                _ => None,
            })
            .flatten()
            .collect();
        let in_branch =
            |start: &NodeId, node: &NodeId| start == node || self.is_ancestor(start, node);
        let mut warned = BTreeSet::new();
        let mut issues = Vec::new();
        for parallel in graph
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Parallel))
        {
            let branches: Vec<&NodeId> = graph.outgoing(&parallel.id).map(|e| &e.to).collect();
            for (i, (a, key)) in writers.iter().enumerate() {
                for (b, other) in writers.iter().skip(i + 1) {
                    if key != other || a == b || self.is_ancestor(a, b) || self.is_ancestor(b, a) {
                        continue;
                    }
                    let apart = branches.iter().any(|x| {
                        in_branch(x, a) && branches.iter().any(|y| y != x && in_branch(y, b))
                    });
                    if apart && warned.insert(((*a).clone(), (*b).clone(), (*key).clone())) {
                        issues.push(GraphIssue::warning(
                            Some(b),
                            format!(
                                "`{a}` sets the variable `{key}` in a parallel branch too; the Join fails if they set different values"
                            ),
                        ));
                    }
                }
            }
        }
        self.issues.extend(issues);
    }

    /// The workflow's declared inputs and outputs: valid, unique keys,
    /// defaults of the declared kind, and outputs some Output node sets.
    fn declarations(&mut self) {
        let mut issues = Vec::new();
        for (what, keys) in [
            (
                "input",
                self.inputs
                    .iter()
                    .map(|i| i.key.as_str())
                    .collect::<Vec<_>>(),
            ),
            (
                "output",
                self.outputs.iter().map(|o| o.key.as_str()).collect(),
            ),
        ] {
            if keys.len() > MAX_WORKFLOW_VALUES {
                issues.push(GraphIssue::error(
                    None,
                    format!("a workflow can have at most {MAX_WORKFLOW_VALUES} {what}s"),
                ));
            }
            let mut seen = BTreeSet::new();
            for key in keys {
                if !is_value_key(key) {
                    issues.push(GraphIssue::error(
                        None,
                        format!(
                            "`{key}` is not a valid {what} key: use 1–64 letters, digits and `_`"
                        ),
                    ));
                } else if !seen.insert(key) {
                    issues.push(GraphIssue::error(
                        None,
                        format!("the {what} `{key}` is declared twice"),
                    ));
                }
            }
        }
        for input in self.inputs {
            if let Some(default) = &input.default {
                if let Err(error) = convert(input.kind, default, self.profile) {
                    issues.push(GraphIssue::error(
                        None,
                        format!("the default of the input `{}` {error}", input.key),
                    ));
                }
                if input.required {
                    issues.push(GraphIssue::warning(
                        None,
                        format!(
                            "the input `{}` is required, so its default is never used",
                            input.key
                        ),
                    ));
                }
            }
        }
        for output in self.outputs {
            let set = self.graph.nodes.iter().any(|n| {
                matches!(&n.kind, NodeKind::Output { values } if values.contains_key(&output.key))
            });
            if !set {
                issues.push(GraphIssue::warning(
                    None,
                    format!(
                        "no Output node sets the output `{}`; callers will not get it",
                        output.key
                    ),
                ));
            }
        }
        self.issues.extend(issues);
    }

    /// A sub-workflow: the called workflow exists, does not lead back here,
    /// is not nested too deeply, and gets a value for every required input
    /// and only for declared ones; sensitive values go to sensitive inputs.
    fn call_node(
        &mut self,
        id: &NodeId,
        owner: WorkflowId,
        workflow: WorkflowId,
        mappings: &BTreeMap<String, Mapping>,
        policy: ErrorPolicy,
    ) {
        self.nested(id, owner, workflow);
        if policy.retry.attempts > 0 {
            self.issues.push(GraphIssue::error(
                Some(id),
                "a sub-workflow cannot be retried",
            ));
        }
        if policy
            .timeout_ms
            .is_some_and(|ms| ms == 0 || ms > MAX_TIMEOUT_MS)
        {
            self.issues.push(GraphIssue::error(
                Some(id),
                "the timeout must be between 1 ms and 10 minutes",
            ));
        }
        let Some(callee) = self.profile.workflow(workflow) else {
            return;
        };
        for (key, mapping) in mappings {
            let sensitive = self.check_mapping(id, mapping);
            match callee.inputs.iter().find(|input| &input.key == key) {
                None => self.issues.push(GraphIssue::error(
                    Some(id),
                    format!("`{key}` is not an input of {}", callee.name),
                )),
                Some(input) if sensitive && !input.sensitive => {
                    self.issues.push(GraphIssue::error(
                        Some(id),
                        format!(
                            "`{key}` gets a sensitive value, but {} does not mark that input sensitive",
                            callee.name
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
        for input in &callee.inputs {
            if input.required && !mappings.contains_key(&input.key) {
                self.issues.push(GraphIssue::error(
                    Some(id),
                    format!(
                        "{} needs a value for its input `{}`",
                        callee.name, input.key
                    ),
                ));
            }
        }
    }

    /// An Output node sets declared outputs; sensitive values only
    /// sensitive ones.
    fn output_node(&mut self, id: &NodeId, values: &BTreeMap<String, Mapping>) {
        if values.is_empty() {
            self.issues
                .push(GraphIssue::warning(Some(id), "this node sets no outputs"));
        }
        for (key, mapping) in values {
            let sensitive = self.check_mapping(id, mapping);
            match self.outputs.iter().find(|o| &o.key == key) {
                None => self.issues.push(GraphIssue::error(
                    Some(id),
                    format!("`{key}` is not an output of this workflow; declare it first"),
                )),
                Some(output) if sensitive && !output.sensitive => {
                    self.issues.push(GraphIssue::error(
                        Some(id),
                        format!(
                            "`{key}` gets a sensitive value; mark the output sensitive so it stays out of history"
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
    }

    /// Recursion and depth of a nested workflow (Loop, Call).
    fn nested(&mut self, id: &NodeId, owner: WorkflowId, workflow: WorkflowId) {
        match nesting(self.profile, workflow, &mut vec![owner]) {
            Err(message) => self.issues.push(GraphIssue::error(Some(id), message)),
            Ok(depth) if depth >= MAX_NESTING => self.issues.push(GraphIssue::error(
                Some(id),
                format!("workflows can be nested at most {MAX_NESTING} levels deep"),
            )),
            Ok(_) => {}
        }
    }

    fn nodes(&mut self, owner: WorkflowId) {
        let graph = self.graph;
        for node in &graph.nodes {
            let id = &node.id;
            match &node.kind {
                NodeKind::Trigger | NodeKind::Parallel | NodeKind::Join | NodeKind::Stop => {}
                NodeKind::Action {
                    action,
                    overrides,
                    mappings,
                    policy,
                } => self.action_node(id, *action, overrides, mappings, *policy),
                NodeKind::Condition { predicate } => match predicate {
                    Predicate::Equals { reference, .. } | Predicate::NotEmpty { reference } => {
                        let reference = reference.clone();
                        self.check_reference(id, &reference);
                    }
                    Predicate::Succeeded { node } => {
                        if !self.is_ancestor(node, id) {
                            self.issues.push(GraphIssue::error(
                                Some(id),
                                format!("`{node}` does not run before this node"),
                            ));
                        }
                    }
                    Predicate::ActiveProject { key } => {
                        if !self.profile.projects.iter().any(|p| p.key.as_str() == key) {
                            self.issues.push(GraphIssue::error(
                                Some(id),
                                format!("this profile has no project `{key}`"),
                            ));
                        }
                    }
                },
                NodeKind::Loop { workflow, times } => {
                    if *times == 0 || *times > MAX_LOOP_ITERATIONS {
                        self.issues.push(GraphIssue::error(
                            Some(id),
                            format!("a loop runs between 1 and {MAX_LOOP_ITERATIONS} times"),
                        ));
                    }
                    self.nested(id, owner, *workflow);
                    if let Some(callee) = self.profile.workflow(*workflow)
                        && callee.inputs.iter().any(|input| input.required)
                    {
                        self.issues.push(GraphIssue::error(
                            Some(id),
                            format!(
                                "{} needs input values; run it with a Sub-workflow node",
                                callee.name
                            ),
                        ));
                    }
                }
                NodeKind::Call {
                    workflow,
                    inputs,
                    policy,
                } => self.call_node(id, owner, *workflow, inputs, *policy),
                NodeKind::Output { values } => self.output_node(id, values),
                NodeKind::SetVariable { values } => self.set_variable_node(id, values),
                NodeKind::Delay { ms } => {
                    if *ms > MAX_DELAY_MS {
                        self.issues
                            .push(GraphIssue::error(Some(id), "a delay is at most 10 minutes"));
                    }
                }
            }
            if !matches!(node.kind, NodeKind::Trigger)
                && graph
                    .trigger()
                    .is_some_and(|t| !self.is_ancestor(&t.id, id))
            {
                self.issues.push(GraphIssue::error(
                    Some(id),
                    "this node cannot be reached from the start node",
                ));
            }
        }
    }

    fn action_node(
        &mut self,
        id: &NodeId,
        action: crate::ActionInstanceId,
        overrides: &crate::ActionParameters,
        mappings: &BTreeMap<String, Mapping>,
        policy: super::ErrorPolicy,
    ) {
        let Some(instance) = self.profile.action(action) else {
            self.issues.push(GraphIssue::error(
                Some(id),
                "the action no longer exists in this profile",
            ));
            return;
        };
        let Some(definition) = self.definition(id, &instance.action_type, &instance.name) else {
            return;
        };
        for key in overrides.iter().map(|(key, _)| key) {
            if !definition.parameters.iter().any(|spec| &spec.key == key) {
                self.issues.push(GraphIssue::error(
                    Some(id),
                    format!("`{key}` is not a parameter of {}", definition.name),
                ));
            }
        }
        self.invocation(
            id,
            definition,
            &instance.parameters.overlay(overrides),
            mappings,
            policy,
        );
    }

    /// The action type, or an issue saying why it cannot run.
    fn definition(
        &mut self,
        id: &NodeId,
        action_type: &crate::ActionTypeId,
        name: &str,
    ) -> Option<&'a crate::ActionType> {
        let registry = self.registry;
        let definition = registry.definition(action_type);
        if definition.is_none() {
            let reason = registry
                .unavailable_reason(action_type)
                .unwrap_or("its action type is not available");
            self.issues
                .push(GraphIssue::error(Some(id), format!("{name}: {reason}")));
        }
        definition
    }

    /// The checks of an action node: the configured parameters, the
    /// mappings (each against the kind of the parameter it fills) and the
    /// error policy.
    fn invocation(
        &mut self,
        id: &NodeId,
        definition: &crate::ActionType,
        configured: &crate::ActionParameters,
        mappings: &BTreeMap<String, Mapping>,
        policy: super::ErrorPolicy,
    ) {
        for key in mappings.keys() {
            if !definition.parameters.iter().any(|spec| &spec.key == key) {
                self.issues.push(GraphIssue::error(
                    Some(id),
                    format!("`{key}` is not a parameter of {}", definition.name),
                ));
            }
        }
        // A required parameter supplied by a mapping is filled in at run
        // time and checked then. For node-output mappings this ends the
        // checks, as it always has, so graphs that ran before keep running;
        // variable references (which no earlier graph contains) are still
        // checked.
        let mut stopped = false;
        if let Err(errors) = definition.normalize(configured) {
            for error in errors {
                if let crate::ParameterError::Missing(key) = &error
                    && mappings.contains_key(key)
                {
                    stopped = true;
                    break;
                }
                self.issues
                    .push(GraphIssue::error(Some(id), error.to_string()));
            }
        }
        for (key, mapping) in mappings {
            if stopped && !reads_variables(mapping) {
                continue;
            }
            match definition.parameters.iter().find(|spec| &spec.key == key) {
                Some(spec) => {
                    self.check_typed_mapping(id, key, mapping, &spec.kind);
                }
                // Reported above; its references are still checked.
                None => {
                    self.check_mapping(id, mapping);
                }
            }
        }
        if stopped {
            return;
        }
        if policy.retry.attempts > MAX_RETRIES {
            self.issues.push(GraphIssue::error(
                Some(id),
                format!("at most {MAX_RETRIES} retries"),
            ));
        }
        if policy.retry.attempts > 0 && !definition.node.retry_safe {
            self.issues.push(GraphIssue::error(
                Some(id),
                format!("{} cannot be retried safely", definition.name),
            ));
        }
        if policy
            .timeout_ms
            .is_some_and(|ms| ms == 0 || ms > MAX_TIMEOUT_MS)
        {
            self.issues.push(GraphIssue::error(
                Some(id),
                "the timeout must be between 1 ms and 10 minutes",
            ));
        }
    }
}

/// How deeply `workflow` nests other workflows through Loop and Call
/// nodes; an error for recursion (direct or through other workflows) or a
/// missing workflow. `path` holds the workflows above.
fn nesting(
    profile: &Profile,
    workflow: WorkflowId,
    path: &mut Vec<WorkflowId>,
) -> Result<usize, String> {
    if path.contains(&workflow) {
        return Err("this would run the workflow inside itself".to_owned());
    }
    let Some(definition) = profile.workflow(workflow) else {
        return Err("the workflow to run does not exist in this profile".to_owned());
    };
    let Some(graph) = &definition.graph else {
        return Ok(1);
    };
    path.push(workflow);
    let mut depth = 1;
    for node in &graph.nodes {
        if let NodeKind::Loop {
            workflow: inner, ..
        }
        | NodeKind::Call {
            workflow: inner, ..
        } = node.kind
        {
            depth = depth.max(1 + nesting(profile, inner, path)?);
        }
    }
    path.pop();
    Ok(depth)
}

/// Whether `mapping` reads anything other than node outputs: a shape that
/// did not exist before variable references.
fn reads_variables(mapping: &Mapping) -> bool {
    mapping
        .references()
        .is_ok_and(|refs| refs.iter().any(|r| !matches!(r, VariableRef::Step(_))))
        || matches!(mapping, Mapping::Variable { .. })
}

/// Whether a value of `value_type` can be inserted into text.
fn renders_as_text(value_type: VariableType) -> bool {
    !matches!(
        value_type,
        VariableType::SecretRef | VariableType::Resource { .. }
    )
}

/// What a parameter of `kind` expects, for messages.
fn expects(kind: &ParameterKind) -> &'static str {
    match kind {
        ParameterKind::Text { .. }
        | ParameterKind::Lines { .. }
        | ParameterKind::Pairs { .. }
        | ParameterKind::Code { .. } => "text",
        ParameterKind::Integer { .. } => "a whole number",
        ParameterKind::Boolean => "true or false",
        ParameterKind::Choice { .. } => "one of its choices",
        ParameterKind::Resource { .. } => "a resource",
        ParameterKind::Project => "a project",
        ParameterKind::Credential => "a credential",
    }
}

fn output_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Text => "text",
        ValueKind::Integer => "a whole number",
        ValueKind::Boolean => "true or false",
        ValueKind::Path => "a path",
        ValueKind::Url => "a URL",
        ValueKind::ProjectKey => "a project key",
    }
}

/// Whether an output of `output` kind can ever convert to a parameter of
/// `kind` (outputs are text; whether a given text converts is known only
/// at run time).
fn output_fits(output: ValueKind, kind: &ParameterKind) -> bool {
    match kind {
        ParameterKind::Integer { .. } => {
            !matches!(output, ValueKind::Boolean | ValueKind::ProjectKey)
        }
        ParameterKind::Boolean => !matches!(output, ValueKind::Integer | ValueKind::ProjectKey),
        ParameterKind::Project => !matches!(output, ValueKind::Integer | ValueKind::Boolean),
        _ => true,
    }
}
