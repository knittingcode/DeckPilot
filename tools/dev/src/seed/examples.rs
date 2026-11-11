//! The development-only "Start Work" and "End Work" workflows: they show
//! the integration modules working together in one graph, using only
//! logical resources, so they run on any machine once its resources are
//! bound. A production installation never contains them.

use deckpilot_core::{
    ActionInstanceId, ActionParameters, ParameterValue, ProjectScope, ResourceRef, Workflow,
};
use deckpilot_modules::{docker, git, notifications, projects, system};

use super::{Builder, Result};

/// The actions both workflows use, created once.
pub(super) struct WorkActions {
    project: ActionInstanceId,
    git_status: ActionInstanceId,
    git_changes: ActionInstanceId,
    docker_status: ActionInstanceId,
    compose_status: ActionInstanceId,
    compose_up: ActionInstanceId,
    compose_down: ActionInstanceId,
    open_editor: ActionInstanceId,
    summary: ActionInstanceId,
}

pub(super) fn actions(b: &mut Builder<'_>) -> Result<WorkActions> {
    let none = ActionParameters::new;
    Ok(WorkActions {
        project: b.action(
            "Active project",
            projects::actions::active_id(),
            &none(),
            "folder-kanban",
            Some("{project.key}"),
        )?,
        git_status: b.action(
            "Git status — current project",
            git::actions::id(git::actions::STATUS),
            &none(),
            "git-branch",
            Some("Git"),
        )?,
        git_changes: b.action(
            "Uncommitted changes — current project",
            git::actions::id(git::actions::CHANGES),
            &none(),
            "list-checks",
            Some("Changes"),
        )?,
        docker_status: b.action(
            "Docker status",
            docker::actions::id(docker::actions::STATUS),
            &none(),
            "container",
            Some("Docker"),
        )?,
        compose_status: b.action(
            "Services — current project",
            docker::actions::id(docker::actions::COMPOSE_STATUS),
            &none(),
            "server",
            Some("Services"),
        )?,
        compose_up: b.action(
            "Start services — current project",
            docker::actions::id(docker::actions::COMPOSE_UP),
            &none(),
            "play",
            Some("Up"),
        )?,
        compose_down: b.action(
            "Stop services — current project",
            docker::actions::id(docker::actions::COMPOSE_DOWN),
            &none(),
            "power",
            Some("Down"),
        )?,
        // The project's `editor` resource: each machine binds its own.
        open_editor: b.action(
            "Open editor — current project",
            system::actions::id(system::actions::OPEN_APP),
            &none().with(
                "application",
                ParameterValue::Resource(ResourceRef::ProjectRole {
                    role: "editor".into(),
                }),
            ),
            "code",
            Some("Editor"),
        )?,
        summary: b.action(
            "Work summary",
            notifications::id(notifications::WORKFLOW_SUMMARY),
            &none(),
            "bell",
            None,
        )?,
    })
}

fn graph(
    nodes: &serde_json::Value,
    edges: &serde_json::Value,
) -> deckpilot_core::graph::WorkflowGraph {
    serde_json::from_value(serde_json::json!({"version": 1, "nodes": nodes, "edges": edges}))
        .expect("the example graph is valid")
}

/// Resolve the project, check Git and Docker, start the project's services
/// when Docker is there, open the editor and show a summary.
pub(super) fn start_work(a: &WorkActions) -> Workflow {
    let tolerant = serde_json::json!({"onError": "continue"});
    let nodes = serde_json::json!([
        {"id": "start", "type": "trigger", "position": {"x": 0, "y": 160}},
        {"id": "project", "type": "action", "action": a.project, "position": {"x": 240, "y": 160}},
        {"id": "git", "type": "action", "action": a.git_status, "label": "Git status",
         "policy": tolerant, "position": {"x": 480, "y": 160}},
        {"id": "docker", "type": "action", "action": a.docker_status, "label": "Docker available?",
         "position": {"x": 720, "y": 160}},
        {"id": "available", "type": "condition", "label": "Docker running",
         "predicate": {"type": "equals", "reference": {"node": "docker", "output": "available"}, "value": "true"},
         "position": {"x": 960, "y": 160}},
        {"id": "services", "type": "action", "action": a.compose_up, "label": "Start services",
         "policy": tolerant, "position": {"x": 1200, "y": 60}},
        {"id": "ready", "type": "join", "position": {"x": 1440, "y": 160}},
        {"id": "editor", "type": "action", "action": a.open_editor, "label": "Open editor",
         "policy": tolerant, "position": {"x": 1680, "y": 160}},
        {"id": "summary", "type": "action", "action": a.summary, "label": "Summary",
         "mappings": {"details": {"type": "output", "reference": {"node": "git", "output": "branch"},
                                  "default": "no branch"}},
         "position": {"x": 1920, "y": 160}},
    ]);
    let edges = serde_json::json!([
        {"from": "start", "to": "project"},
        {"from": "project", "to": "git"},
        {"from": "git", "to": "docker"},
        {"from": "docker", "to": "available"},
        {"from": "available", "port": "true", "to": "services"},
        {"from": "available", "port": "false", "to": "ready"},
        {"from": "services", "to": "ready"},
        {"from": "ready", "to": "editor"},
        {"from": "editor", "to": "summary"},
    ]);
    let mut workflow = Workflow::new("Start Work");
    workflow.description =
        "Check Git and Docker, start the project's services, open the editor, summarize".into();
    workflow.project_scope = Some(ProjectScope::Execution);
    workflow.graph = Some(graph(&nodes, &edges));
    workflow
}

/// Show uncommitted changes, look at the project's services and stop them
/// when the project has them, then show a summary.
pub(super) fn end_work(a: &WorkActions) -> Workflow {
    let tolerant = serde_json::json!({"onError": "continue"});
    let nodes = serde_json::json!([
        {"id": "start", "type": "trigger", "position": {"x": 0, "y": 160}},
        {"id": "project", "type": "action", "action": a.project, "position": {"x": 240, "y": 160}},
        {"id": "changes", "type": "action", "action": a.git_changes, "label": "Uncommitted changes",
         "policy": tolerant, "position": {"x": 480, "y": 160}},
        {"id": "services", "type": "action", "action": a.compose_status, "label": "Services",
         "policy": tolerant, "position": {"x": 720, "y": 160}},
        {"id": "configured", "type": "condition", "label": "Project has services",
         "predicate": {"type": "succeeded", "node": "services"},
         "position": {"x": 960, "y": 160}},
        {"id": "stop", "type": "action", "action": a.compose_down, "label": "Stop services",
         "policy": tolerant, "position": {"x": 1200, "y": 60}},
        {"id": "done", "type": "join", "position": {"x": 1440, "y": 160}},
        {"id": "summary", "type": "action", "action": a.summary, "label": "Summary",
         "mappings": {"details": {"type": "output", "reference": {"node": "changes", "output": "files"},
                                  "default": "no uncommitted changes"}},
         "position": {"x": 1680, "y": 160}},
    ]);
    let edges = serde_json::json!([
        {"from": "start", "to": "project"},
        {"from": "project", "to": "changes"},
        {"from": "changes", "to": "services"},
        {"from": "services", "to": "configured"},
        {"from": "configured", "port": "true", "to": "stop"},
        {"from": "configured", "port": "false", "to": "done"},
        {"from": "stop", "to": "done"},
        {"from": "done", "to": "summary"},
    ]);
    let mut workflow = Workflow::new("End Work");
    workflow.description =
        "List uncommitted changes, stop the project's services if it has them, summarize".into();
    workflow.project_scope = Some(ProjectScope::Execution);
    workflow.graph = Some(graph(&nodes, &edges));
    workflow
}
