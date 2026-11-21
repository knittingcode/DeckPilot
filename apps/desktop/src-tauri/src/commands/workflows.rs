use tauri::State;

use deckpilot_core::graph::{Conversion, GraphIssue, from_steps};
use deckpilot_core::{ProfileId, Workflow, WorkflowId};

use super::reload::Saved;
use super::{modify_profile, modify_profile_and_reload};
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, WORKFLOWS_PLUGIN, load_profile};

/// Workflow management belongs to the Workflows plugin
/// (`deckpilot.workflows`, decisions D1/D6 of 2026-11-20) and is refused
/// while it is off, except for step workflows without a graph: those are
/// the key editor's Multi Actions, a core feature that keeps working with
/// the plugin off. Keys that run workflows run them either way (the
/// engine stays in the core), and no workflow is ever deleted because the
/// plugin is off.
fn require_workflows(state: &AppState, workflow: Option<&Workflow>) -> CommandResult<()> {
    if workflow.is_some_and(|w| w.graph.is_none()) {
        return Ok(());
    }
    state.require_feature(WORKFLOWS_PLUGIN)
}

/// Problems of a workflow's graph, as the engine sees them, for the editor
/// to show on the nodes. The workflow does not have to be saved.
#[tauri::command]
pub fn validate_workflow_graph(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow: Workflow,
) -> CommandResult<Vec<GraphIssue>> {
    state.require_feature(WORKFLOWS_PLUGIN)?;
    let profile = load_profile(&*state.store()?, profile_id)?;
    Ok(workflow.graph_issues(&profile, &state.actions()))
}

/// The graph form of a workflow's steps, with notes where it behaves
/// differently. Nothing is saved: the editor stores the graph when the user
/// saves, and the steps are kept.
#[tauri::command]
pub fn convert_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow: Workflow,
) -> CommandResult<Conversion> {
    state.require_feature(WORKFLOWS_PLUGIN)?;
    let profile = load_profile(&*state.store()?, profile_id)?;
    Ok(from_steps(&workflow, &profile))
}

/// Creates or replaces a workflow after validating it. Returns once the
/// agent has read it, so it can be started right away. A graph workflow
/// needs the Workflows plugin, and so does replacing one; a step workflow
/// (Multi Action) does not.
#[tauri::command]
pub async fn save_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow: Workflow,
) -> CommandResult<Saved<Workflow>> {
    require_workflows(&state, Some(&workflow))?;
    if workflow.graph.is_none() {
        // Turning a graph back into steps is managing a workflow too.
        let profile = load_profile(&*state.store()?, profile_id)?;
        if let Some(existing) = profile.workflow(workflow.id) {
            require_workflows(&state, Some(existing))?;
        }
    }
    let mut workflow = workflow;
    super::trim_in_place(&mut workflow.name);
    super::trim_in_place(&mut workflow.description);
    modify_profile_and_reload(&state, profile_id, |profile| {
        let issues = workflow.validate(profile, &state.actions());
        if !issues.is_empty() {
            return Err(CommandError::Invalid(issues.join("\n")));
        }
        match profile.workflows.iter_mut().find(|w| w.id == workflow.id) {
            Some(existing) => *existing = workflow.clone(),
            None => profile.workflows.push(workflow.clone()),
        }
        Ok(workflow)
    })
    .await
}

#[tauri::command]
pub fn duplicate_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow_id: WorkflowId,
    name: String,
) -> CommandResult<Workflow> {
    modify_profile(&state, profile_id, |profile| {
        let mut copy = profile
            .workflow(workflow_id)
            .ok_or(CommandError::NotFound("workflow"))?
            .clone();
        require_workflows(&state, Some(&copy))?;
        copy.id = WorkflowId::new();
        name.trim().clone_into(&mut copy.name);
        if copy.name.is_empty() {
            return Err(CommandError::invalid(
                "workflowNameEmpty",
                "workflow name must not be empty",
            ));
        }
        profile.workflows.push(copy.clone());
        Ok(copy)
    })
}

/// Deletes a workflow that no layout uses: a graph workflow only while
/// the Workflows plugin is on, a step workflow (Multi Action) always.
#[tauri::command]
pub fn delete_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow_id: WorkflowId,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        require_workflows(&state, profile.workflow(workflow_id))?;
        profile.delete_workflow(workflow_id)?;
        Ok(())
    })
}
