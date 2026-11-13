//! Safe deletion of actions: every place that uses an action instance, and
//! the explicit operations that remove those uses (replace with another
//! action, unbind) before the action itself can be deleted.
//!
//! Nothing here deletes a binding on its own: [`delete_action`] refuses while
//! anything refers to the action and lists what does; only
//! [`replace_action`] and [`unbind_action`], which the user chooses per
//! reference, change bindings, steps or nodes.

use serde::{Deserialize, Serialize};

use deckpilot_core::graph::{NodeId, NodeKind};
use deckpilot_core::trigger::TriggerKind;
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, BindingTarget, LayoutId, PageId, Profile,
    Trigger, WorkflowId,
};
use deckpilot_device::ControlId;

use crate::{Result, StorageError};

/// One place that uses an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReferenceSite {
    /// A key, dial or touch binding of a layout page.
    Binding {
        layout: LayoutId,
        page: PageId,
        control: ControlId,
        trigger: Trigger,
    },
    /// A step of a step-list workflow (index from 0).
    Step { workflow: WorkflowId, index: usize },
    /// An action node of a graph workflow.
    Node { workflow: WorkflowId, node: NodeId },
    /// A trigger (schedule, lifecycle event) of a workflow that runs the
    /// action through a step or node. Indirect: it goes away once the
    /// workflow no longer uses the action, and it is never changed here.
    Trigger { workflow: WorkflowId, index: usize },
}

impl ReferenceSite {
    /// Whether replacing or unbinding acts on this site itself.
    pub fn is_direct(&self) -> bool {
        !matches!(self, Self::Trigger { .. })
    }
}

/// A use of an action, with names for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionReference {
    pub site: ReferenceSite,
    /// The layout or workflow name.
    pub owner_name: String,
    /// The page name of a binding, the node label of a node, the trigger
    /// kind of a trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The trigger of a [`ReferenceSite::Trigger`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TriggerKind>,
}

fn targets(target: &BindingTarget, id: ActionInstanceId) -> bool {
    target.action_id() == Some(id)
}

/// Every place in `profile` that uses action `id`: layout bindings,
/// workflow steps and graph nodes, and the triggers of the workflows that
/// use it.
pub fn action_references(profile: &Profile, id: ActionInstanceId) -> Vec<ActionReference> {
    let mut references = Vec::new();
    for layout in &profile.layouts {
        for page in &layout.pages {
            for binding in page.bindings.iter().filter(|b| targets(&b.target, id)) {
                references.push(ActionReference {
                    site: ReferenceSite::Binding {
                        layout: layout.id,
                        page: page.id,
                        control: binding.control,
                        trigger: binding.trigger,
                    },
                    owner_name: layout.name.clone(),
                    detail: Some(page.name.clone()),
                    trigger: None,
                });
            }
        }
    }
    for workflow in &profile.workflows {
        let before = references.len();
        for (index, step) in workflow.steps.iter().enumerate() {
            if step.action == id {
                references.push(ActionReference {
                    site: ReferenceSite::Step {
                        workflow: workflow.id,
                        index,
                    },
                    owner_name: workflow.name.clone(),
                    detail: None,
                    trigger: None,
                });
            }
        }
        for node in workflow.graph.iter().flat_map(|graph| &graph.nodes) {
            if matches!(&node.kind, NodeKind::Action { action, .. } if *action == id) {
                references.push(ActionReference {
                    site: ReferenceSite::Node {
                        workflow: workflow.id,
                        node: node.id.clone(),
                    },
                    owner_name: workflow.name.clone(),
                    detail: (!node.label.is_empty()).then(|| node.label.clone()),
                    trigger: None,
                });
            }
        }
        if references.len() > before {
            for (index, trigger) in workflow.triggers.iter().enumerate() {
                references.push(ActionReference {
                    site: ReferenceSite::Trigger {
                        workflow: workflow.id,
                        index,
                    },
                    owner_name: workflow.name.clone(),
                    detail: workflow.triggers_paused.then(|| "paused".to_owned()),
                    trigger: Some(trigger.kind.clone()),
                });
            }
        }
    }
    references
}

/// Which uses an operation acts on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "sites", rename_all = "camelCase")]
pub enum Selection {
    /// Every direct use.
    #[default]
    All,
    /// These sites only; each must be a current direct use of the action.
    Sites(Vec<ReferenceSite>),
}

/// What [`replace_action`] or [`unbind_action`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceChange {
    /// Sites that were changed.
    pub changed: Vec<ReferenceSite>,
    /// Bindings, steps and nodes whose parameter overrides (and mappings)
    /// were removed because the replacement has another action type.
    pub cleared_settings: Vec<ReferenceSite>,
}

fn require_action(profile: &Profile, id: ActionInstanceId) -> Result<&ActionInstance> {
    profile
        .action(id)
        .ok_or_else(|| StorageError::Invalid("the action does not exist in this profile".into()))
}

/// The direct sites to act on, checked against the current uses.
fn selected(
    profile: &Profile,
    id: ActionInstanceId,
    selection: &Selection,
) -> Result<Vec<ReferenceSite>> {
    let current: Vec<ReferenceSite> = action_references(profile, id)
        .into_iter()
        .map(|reference| reference.site)
        .filter(ReferenceSite::is_direct)
        .collect();
    match selection {
        Selection::All => Ok(current),
        Selection::Sites(sites) => {
            for site in sites {
                if !site.is_direct() {
                    return Err(StorageError::Invalid(
                        "a trigger follows its workflow; change the workflow's step or node".into(),
                    ));
                }
                if !current.contains(site) {
                    return Err(StorageError::Invalid(
                        "a selected place no longer uses this action; reload and try again".into(),
                    ));
                }
            }
            Ok(sites.clone())
        }
    }
}

/// Points the selected uses of action `from` at action `to`. Overrides of
/// bindings and steps, and overrides and mappings of nodes, are kept when
/// both have the same action type and removed otherwise (they belong to
/// the old type's parameters); the result lists where.
pub fn replace_action(
    profile: &mut Profile,
    from: ActionInstanceId,
    to: ActionInstanceId,
    selection: &Selection,
) -> Result<ReferenceChange> {
    if from == to {
        return Err(StorageError::Invalid(
            "an action cannot replace itself".into(),
        ));
    }
    let same_type =
        require_action(profile, from)?.action_type == require_action(profile, to)?.action_type;
    let sites = selected(profile, from, selection)?;
    let mut change = ReferenceChange::default();
    for site in sites {
        match &site {
            ReferenceSite::Binding {
                layout,
                page,
                control,
                trigger,
            } => {
                for binding in profile
                    .layouts
                    .iter_mut()
                    .filter(|l| l.id == *layout)
                    .flat_map(|l| &mut l.pages)
                    .filter(|p| p.id == *page)
                    .flat_map(|p| &mut p.bindings)
                    .filter(|b| b.control == *control && b.trigger == *trigger)
                {
                    if let BindingTarget::Action { action, overrides } = &mut binding.target
                        && *action == from
                    {
                        *action = to;
                        if !same_type && !overrides.is_empty() {
                            *overrides = ActionParameters::new();
                            change.cleared_settings.push(site.clone());
                        }
                    }
                }
            }
            ReferenceSite::Step { workflow, index } => {
                if let Some(step) = profile
                    .workflows
                    .iter_mut()
                    .find(|w| w.id == *workflow)
                    .and_then(|w| w.steps.get_mut(*index))
                {
                    step.action = to;
                    if !same_type && !step.overrides.is_empty() {
                        step.overrides = ActionParameters::new();
                        change.cleared_settings.push(site.clone());
                    }
                }
            }
            ReferenceSite::Node { workflow, node } => {
                if let Some(NodeKind::Action {
                    action,
                    overrides,
                    mappings,
                    ..
                }) = profile
                    .workflows
                    .iter_mut()
                    .find(|w| w.id == *workflow)
                    .and_then(|w| w.graph.as_mut())
                    .and_then(|g| g.nodes.iter_mut().find(|n| n.id == *node))
                    .map(|n| &mut n.kind)
                {
                    *action = to;
                    if !same_type && (!overrides.is_empty() || !mappings.is_empty()) {
                        *overrides = ActionParameters::new();
                        mappings.clear();
                        change.cleared_settings.push(site.clone());
                    }
                }
            }
            ReferenceSite::Trigger { .. } => {}
        }
        change.changed.push(site);
    }
    Ok(change)
}

/// Removes the selected uses of action `id`: the key bindings are cleared
/// (the control becomes empty) and the steps are removed. Graph nodes are
/// part of a connected graph and are refused: replace them, or remove them
/// in the workflow editor.
pub fn unbind_action(
    profile: &mut Profile,
    id: ActionInstanceId,
    selection: &Selection,
) -> Result<ReferenceChange> {
    require_action(profile, id)?;
    let sites = selected(profile, id, selection)?;
    if sites
        .iter()
        .any(|site| matches!(site, ReferenceSite::Node { .. }))
    {
        return Err(StorageError::Invalid(
            "a workflow node cannot be unbound; replace it with another action or remove it in the workflow editor"
                .into(),
        ));
    }
    // Steps are removed from the highest index down, so that the indexes
    // of the others stay valid.
    let mut steps: Vec<(WorkflowId, usize)> = sites
        .iter()
        .filter_map(|site| match site {
            ReferenceSite::Step { workflow, index } => Some((*workflow, *index)),
            _ => None,
        })
        .collect();
    steps.sort_by_key(|step| std::cmp::Reverse(step.1));
    for (workflow, index) in steps {
        if let Some(workflow) = profile.workflows.iter_mut().find(|w| w.id == workflow)
            && index < workflow.steps.len()
        {
            workflow.steps.remove(index);
        }
    }
    for site in &sites {
        if let ReferenceSite::Binding {
            layout,
            page,
            control,
            trigger,
        } = site
        {
            for page in profile
                .layouts
                .iter_mut()
                .filter(|l| l.id == *layout)
                .flat_map(|l| &mut l.pages)
                .filter(|p| p.id == *page)
            {
                page.bindings.retain(|b| {
                    !(b.control == *control && b.trigger == *trigger && targets(&b.target, id))
                });
            }
        }
    }
    Ok(ReferenceChange {
        changed: sites,
        cleared_settings: Vec::new(),
    })
}

/// Deletes action `id` if nothing uses it; otherwise refuses with every use
/// ([`StorageError::ActionInUse`]) and changes nothing.
pub fn delete_action(profile: &mut Profile, id: ActionInstanceId) -> Result<ActionInstance> {
    require_action(profile, id)?;
    let references = action_references(profile, id);
    if !references.is_empty() {
        return Err(StorageError::ActionInUse(references));
    }
    let index = profile
        .actions
        .iter()
        .position(|action| action.id == id)
        .ok_or_else(|| StorageError::Invalid("the action does not exist in this profile".into()))?;
    Ok(profile.actions.remove(index))
}

#[cfg(test)]
mod tests;
