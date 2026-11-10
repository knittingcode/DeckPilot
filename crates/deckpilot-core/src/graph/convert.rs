//! Conversion of a sequential workflow (steps) into a graph.
//!
//! Workflows are never converted automatically: a workflow without a graph
//! keeps running with the sequential engine. The editor converts when the
//! user opens such a workflow, shows the notes, and the graph is stored only
//! when the user saves it. The step list is kept unchanged.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    Edge, ErrorPolicy, Node, NodeId, NodeKind, OnError, Port, Position, Predicate, WorkflowGraph,
};
use crate::{OnFailure, Profile, StepCondition, Workflow};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversion {
    pub graph: WorkflowGraph,
    /// Where the graph behaves differently from the steps, for the user to
    /// review. Empty when the conversion is exact.
    pub notes: Vec<String>,
}

const SPACING: f64 = 280.0;

fn id(text: String) -> NodeId {
    NodeId::new(text).expect("generated node ids are valid")
}

/// The graph equivalent of `workflow`'s steps.
pub fn from_steps(workflow: &Workflow, profile: &Profile) -> Conversion {
    let mut graph = WorkflowGraph::default();
    let mut notes = Vec::new();
    let mut column = 0.0;
    let mut place = |kind: NodeKind, node_id: NodeId, row: f64| {
        let node = Node {
            id: node_id,
            kind,
            label: String::new(),
            position: Position {
                x: column * SPACING,
                y: row * 140.0,
            },
        };
        column += 1.0;
        node
    };

    let start = id("start".to_owned());
    graph
        .nodes
        .push(place(NodeKind::Trigger, start.clone(), 0.0));
    // Where the next step connects: a node and its continuation port.
    let mut tail = (start, Port::Out);
    let mut previous_step: Option<(NodeId, StepCondition)> = None;

    for (index, step) in workflow.steps.iter().enumerate() {
        let position = index + 1;
        let step_id = id(format!("step_{position}"));
        let action = Node {
            label: profile
                .action(step.action)
                .map(|a| a.name.clone())
                .unwrap_or_default(),
            ..place(action_kind(step), step_id.clone(), 0.0)
        };

        let predicate = condition_for(
            step.condition,
            previous_step.as_ref(),
            position,
            profile,
            &mut notes,
        );

        match predicate {
            None => {
                graph.edges.push(Edge {
                    from: tail.0.clone(),
                    port: tail.1,
                    to: step_id.clone(),
                });
                graph.nodes.push(action);
                tail = (step_id.clone(), Port::Out);
            }
            Some((predicate, inverted)) => {
                // check → step → join, and check → join when not met.
                let check = id(format!("check_{position}"));
                let join = id(format!("after_{position}"));
                graph
                    .nodes
                    .push(place(NodeKind::Condition { predicate }, check.clone(), 0.0));
                graph.edges.push(Edge {
                    from: tail.0.clone(),
                    port: tail.1,
                    to: check.clone(),
                });
                let (run, skip) = if inverted {
                    (Port::False, Port::True)
                } else {
                    (Port::True, Port::False)
                };
                graph.edges.push(Edge {
                    from: check.clone(),
                    port: run,
                    to: step_id.clone(),
                });
                graph.nodes.push(action);
                graph.nodes.push(place(NodeKind::Join, join.clone(), 0.0));
                graph.edges.push(Edge {
                    from: step_id.clone(),
                    port: Port::Out,
                    to: join.clone(),
                });
                graph.edges.push(Edge {
                    from: check,
                    port: skip,
                    to: join.clone(),
                });
                tail = (join, Port::Out);
            }
        }
        previous_step = Some((step_id, step.condition));
    }
    Conversion { graph, notes }
}

/// The predicate that replaces a step condition, and whether its result is
/// inverted (`PreviousFailed`). `None`: the step always runs.
fn condition_for(
    condition: StepCondition,
    previous: Option<&(NodeId, StepCondition)>,
    position: usize,
    profile: &Profile,
    notes: &mut Vec<String>,
) -> Option<(Predicate, bool)> {
    match condition {
        StepCondition::Always => None,
        StepCondition::PreviousSucceeded | StepCondition::PreviousFailed => {
            let Some((previous, previous_condition)) = previous else {
                notes.push(format!(
                    "step {position}: depends on a previous step but is the first; it now always runs"
                ));
                return None;
            };
            if *previous_condition != StepCondition::Always {
                notes.push(format!(
                    "step {position}: the step before it is conditional; the graph checks step {} itself, while the steps checked the last step that ran",
                    position - 1
                ));
            }
            Some((
                Predicate::Succeeded {
                    node: previous.clone(),
                },
                condition == StepCondition::PreviousFailed,
            ))
        }
        StepCondition::ActiveProject { project } => {
            if let Some(project) = profile.project(project) {
                Some((
                    Predicate::ActiveProject {
                        key: project.key.as_str().to_owned(),
                    },
                    false,
                ))
            } else {
                notes.push(format!(
                    "step {position}: the project of its condition no longer exists; it now always runs"
                ));
                None
            }
        }
    }
}

/// The action node of a step: same action, overrides and failure handling.
fn action_kind(step: &crate::WorkflowStep) -> NodeKind {
    NodeKind::Action {
        action: step.action,
        overrides: step.overrides.clone(),
        mappings: BTreeMap::default(),
        policy: ErrorPolicy {
            on_error: match step.on_failure {
                OnFailure::Stop => OnError::Stop,
                OnFailure::Continue => OnError::Continue,
            },
            ..ErrorPolicy::default()
        },
    }
}
