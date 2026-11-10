//! Structural and semantic checks of a workflow graph. They run before a
//! graph is saved and again before every run, so a graph that became
//! invalid (a module was disabled, an action deleted) is never executed.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::mapping::references_in;
use super::{
    GRAPH_VERSION, MAX_DELAY_MS, MAX_LOOP_ITERATIONS, MAX_NESTING, MAX_NODES, MAX_RETRIES,
    MAX_TIMEOUT_MS, Mapping, NodeId, NodeKind, Port, Predicate, Reference, WorkflowGraph,
};
use crate::{ActionRegistry, Profile, WorkflowId};

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
}

impl WorkflowGraph {
    /// Every problem of the graph of workflow `owner` in `profile`. The graph
    /// may run if no issue is an [`IssueSeverity::Error`].
    pub fn validate(
        &self,
        owner: WorkflowId,
        profile: &Profile,
        registry: &ActionRegistry,
    ) -> Vec<GraphIssue> {
        let mut checker = Checker {
            graph: self,
            profile,
            registry,
            issues: Vec::new(),
            ancestors: BTreeMap::new(),
        };
        // Semantic checks need an acyclic graph whose connections all refer
        // to existing nodes; other structural problems do not hide them.
        if checker.structure() {
            checker.compute_ancestors();
            checker.nodes(owner);
            checker.parallel_regions();
        }
        checker.issues
    }

    /// Whether the graph may run.
    pub fn is_runnable(issues: &[GraphIssue]) -> bool {
        issues.iter().all(|i| i.severity != IssueSeverity::Error)
    }
}

impl Checker<'_> {
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

    fn check_reference(&mut self, node: &NodeId, reference: &Reference) {
        let graph = self.graph;
        if !self.is_ancestor(&reference.node, node) {
            self.issues.push(GraphIssue::error(
                Some(node),
                format!(
                    "{reference}: `{}` does not run before this node",
                    reference.node
                ),
            ));
            return;
        }
        let provides = graph
            .node(&reference.node)
            .and_then(|source| match &source.kind {
                NodeKind::Action { action, .. } => {
                    let instance = self.profile.action(*action)?;
                    let definition = self.registry.definition(&instance.action_type)?;
                    Some(
                        definition
                            .node
                            .outputs
                            .iter()
                            .any(|o| o.key == reference.output),
                    )
                }
                _ => Some(false),
            });
        if provides == Some(false) {
            self.issues.push(GraphIssue::error(
                Some(node),
                format!(
                    "{reference}: `{}` provides no output `{}`",
                    reference.node, reference.output
                ),
            ));
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
                    match nesting(self.profile, *workflow, &mut vec![owner]) {
                        Err(message) => self.issues.push(GraphIssue::error(Some(id), message)),
                        Ok(depth) if depth >= MAX_NESTING => self.issues.push(GraphIssue::error(
                            Some(id),
                            format!("workflows can be nested at most {MAX_NESTING} levels deep"),
                        )),
                        Ok(_) => {}
                    }
                }
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
        let Some(definition) = self.registry.definition(&instance.action_type) else {
            let reason = self
                .registry
                .unavailable_reason(&instance.action_type)
                .unwrap_or("its action type is not available");
            self.issues.push(GraphIssue::error(
                Some(id),
                format!("{}: {reason}", instance.name),
            ));
            return;
        };
        for key in overrides.iter().map(|(key, _)| key).chain(mappings.keys()) {
            if !definition.parameters.iter().any(|spec| &spec.key == key) {
                self.issues.push(GraphIssue::error(
                    Some(id),
                    format!("`{key}` is not a parameter of {}", definition.name),
                ));
            }
        }
        if let Err(errors) = definition.normalize(&instance.parameters.overlay(overrides)) {
            // A required parameter supplied by a mapping is filled
            // in at run time and checked then.
            for error in errors {
                if let crate::ParameterError::Missing(key) = &error
                    && mappings.contains_key(key)
                {
                    return;
                }
                self.issues
                    .push(GraphIssue::error(Some(id), error.to_string()));
            }
        }
        for mapping in mappings.values() {
            let references = match mapping {
                Mapping::Output { reference, .. } => Ok(vec![reference.clone()]),
                Mapping::Template { template } => references_in(template),
            };
            match references {
                Ok(references) => {
                    for reference in references {
                        self.check_reference(id, &reference);
                    }
                }
                Err(error) => {
                    self.issues
                        .push(GraphIssue::error(Some(id), error.to_string()));
                }
            }
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

    /// Actions that change the active project may not run while parallel
    /// branches are open: the order of their effects would be undefined.
    fn parallel_regions(&mut self) {
        let graph = self.graph;
        let Some(order) = graph.topological_order() else {
            return;
        };
        let mut open: BTreeMap<&NodeId, usize> = BTreeMap::new();
        for id in order {
            let Some(node) = graph.node(id) else { continue };
            let inside = graph
                .incoming(id)
                .map(|edge| open.get(&edge.from).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);
            let after = match node.kind {
                NodeKind::Parallel => inside + 1,
                NodeKind::Join => inside.saturating_sub(1),
                _ => inside,
            };
            open.insert(id, after);
            if inside > 0
                && let NodeKind::Action { action, .. } = &node.kind
                && let Some(instance) = self.profile.action(*action)
                && let Some(definition) = self.registry.definition(&instance.action_type)
                && definition.node.changes_context
            {
                self.issues.push(GraphIssue::error(
                    Some(id),
                    format!(
                        "{} changes the active project and cannot run in parallel branches",
                        definition.name
                    ),
                ));
            }
        }
    }
}

/// How deeply `workflow` nests other workflows through Loop nodes; an error
/// for recursion or a missing workflow. `path` holds the workflows above.
fn nesting(
    profile: &Profile,
    workflow: WorkflowId,
    path: &mut Vec<WorkflowId>,
) -> Result<usize, String> {
    if path.contains(&workflow) {
        return Err("the loop would run the workflow inside itself".to_owned());
    }
    let Some(definition) = profile.workflow(workflow) else {
        return Err("the loop refers to a workflow that does not exist in this profile".to_owned());
    };
    let Some(graph) = &definition.graph else {
        return Ok(1);
    };
    path.push(workflow);
    let mut depth = 1;
    for node in &graph.nodes {
        if let NodeKind::Loop {
            workflow: inner, ..
        } = node.kind
        {
            depth = depth.max(1 + nesting(profile, inner, path)?);
        }
    }
    path.pop();
    Ok(depth)
}
