use tauri::State;

use deckpilot_core::graph::{Conversion, GraphIssue, from_steps};
use deckpilot_core::{ProfileId, Workflow, WorkflowId};

use super::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile};

/// Problems of a workflow's graph, as the engine sees them, for the editor
/// to show on the nodes. The workflow does not have to be saved.
#[tauri::command]
pub fn validate_workflow_graph(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow: Workflow,
) -> CommandResult<Vec<GraphIssue>> {
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
    let profile = load_profile(&*state.store()?, profile_id)?;
    Ok(from_steps(&workflow, &profile))
}

/// Creates or replaces a workflow after validating it.
#[tauri::command]
pub fn save_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow: Workflow,
) -> CommandResult<Workflow> {
    let mut workflow = workflow;
    super::trim_in_place(&mut workflow.name);
    super::trim_in_place(&mut workflow.description);
    modify_profile(&state, profile_id, |profile| {
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
        copy.id = WorkflowId::new();
        name.trim().clone_into(&mut copy.name);
        if copy.name.is_empty() {
            return Err(CommandError::Invalid(
                "workflow name must not be empty".into(),
            ));
        }
        profile.workflows.push(copy.clone());
        Ok(copy)
    })
}

/// Deletes a workflow that no layout uses.
#[tauri::command]
pub fn delete_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow_id: WorkflowId,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        profile.delete_workflow(workflow_id)?;
        Ok(())
    })
}
