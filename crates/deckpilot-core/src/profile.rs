//! Profiles: the portable unit of user configuration.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use deckpilot_device::{ControlId, DeviceModelId, ModelCatalog};

use crate::{
    ActionInstance, ActionInstanceId, BindingTarget, DeviceLayout, LayoutId, LayoutIssue, PageId,
    ProfileId, Project, ProjectId, StepCondition, Workflow, WorkflowId,
};

/// A self-contained set of actions, workflows, projects and layouts.
///
/// Profiles contain no machine-specific data and can be moved between
/// computers and operating systems.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub actions: Vec<ActionInstance>,
    #[serde(default)]
    pub workflows: Vec<Workflow>,
    /// Ordered list; the order defines the project cycle.
    #[serde(default)]
    pub projects: Vec<Project>,
    /// Project selected when the profile is activated for the first time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_project: Option<ProjectId>,
    #[serde(default)]
    pub layouts: Vec<DeviceLayout>,
    /// The layout used for each device model. Models without an entry use
    /// their first layout.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub active_layouts: BTreeMap<DeviceModelId, LayoutId>,
}

/// Where an action or workflow is referenced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Usage {
    WorkflowStep {
        workflow: WorkflowId,
        workflow_name: String,
        position: usize,
    },
    LayoutBinding {
        layout: LayoutId,
        layout_name: String,
        page: PageId,
        page_name: String,
        control: ControlId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProfileIssue {
    #[error("profile name must not be empty")]
    EmptyName,
    #[error("project key `{0}` is used more than once")]
    DuplicateProjectKey(String),
    #[error("workflow `{workflow}` references a missing action")]
    MissingWorkflowAction {
        workflow: String,
        action: ActionInstanceId,
    },
    #[error("layout `{layout}`: {issue}")]
    Layout { layout: String, issue: LayoutIssue },
    #[error("it is still used in {0} place(s); remove those references first")]
    InUse(usize),
    #[error("{0} does not exist in this profile")]
    NotFound(&'static str),
    #[error("the layout is for a different device model")]
    ModelMismatch,
}

impl Profile {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ProfileId::new(),
            name: name.into(),
            description: String::new(),
            actions: Vec::new(),
            workflows: Vec::new(),
            projects: Vec::new(),
            default_project: None,
            layouts: Vec::new(),
            active_layouts: BTreeMap::new(),
        }
    }

    pub fn action(&self, id: ActionInstanceId) -> Option<&ActionInstance> {
        self.actions.iter().find(|action| action.id == id)
    }

    pub fn workflow(&self, id: WorkflowId) -> Option<&Workflow> {
        self.workflows.iter().find(|workflow| workflow.id == id)
    }

    pub fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.iter().find(|project| project.id == id)
    }

    pub fn layout(&self, id: LayoutId) -> Option<&DeviceLayout> {
        self.layouts.iter().find(|layout| layout.id == id)
    }

    pub fn project_by_key(&self, key: &str) -> Option<&Project> {
        self.projects
            .iter()
            .find(|project| project.key.matches(key))
    }

    /// Adds a project unless its key is already taken.
    pub fn add_project(&mut self, project: Project) -> Result<(), ProfileIssue> {
        if self.project_by_key(project.key.as_str()).is_some() {
            return Err(ProfileIssue::DuplicateProjectKey(project.key.to_string()));
        }
        self.projects.push(project);
        Ok(())
    }

    /// Replaces a project, keeping its position in the cycle.
    pub fn update_project(&mut self, project: Project) -> Result<(), ProfileIssue> {
        let clash = self
            .projects
            .iter()
            .any(|p| p.id != project.id && p.key.matches(project.key.as_str()));
        if clash {
            return Err(ProfileIssue::DuplicateProjectKey(project.key.to_string()));
        }
        let slot = self
            .projects
            .iter_mut()
            .find(|p| p.id == project.id)
            .ok_or(ProfileIssue::NotFound("project"))?;
        *slot = project;
        Ok(())
    }

    /// Moves a project to `index` in the cycle order (clamped to the end).
    pub fn move_project(&mut self, id: ProjectId, index: usize) -> Result<(), ProfileIssue> {
        let from = self
            .projects
            .iter()
            .position(|p| p.id == id)
            .ok_or(ProfileIssue::NotFound("project"))?;
        let project = self.projects.remove(from);
        let to = index.min(self.projects.len());
        self.projects.insert(to, project);
        Ok(())
    }

    /// Removes a project unless a workflow condition refers to it.
    pub fn delete_project(&mut self, id: ProjectId) -> Result<Project, ProfileIssue> {
        let referenced = self
            .workflows
            .iter()
            .flat_map(|w| &w.steps)
            .filter(|step| step.condition == StepCondition::ActiveProject { project: id })
            .count();
        if referenced > 0 {
            return Err(ProfileIssue::InUse(referenced));
        }
        let index = self
            .projects
            .iter()
            .position(|p| p.id == id)
            .ok_or(ProfileIssue::NotFound("project"))?;
        if self.default_project == Some(id) {
            self.default_project = None;
        }
        Ok(self.projects.remove(index))
    }

    fn target_usages(&self, matches: impl Fn(&BindingTarget) -> bool) -> Vec<Usage> {
        let mut usages = Vec::new();
        for layout in &self.layouts {
            for page in &layout.pages {
                for binding in page.bindings.iter().filter(|b| matches(&b.target)) {
                    usages.push(Usage::LayoutBinding {
                        layout: layout.id,
                        layout_name: layout.name.clone(),
                        page: page.id,
                        page_name: page.name.clone(),
                        control: binding.control,
                    });
                }
            }
        }
        usages
    }

    /// Workflow steps and layout bindings that use an action.
    pub fn action_usages(&self, id: ActionInstanceId) -> Vec<Usage> {
        let mut usages: Vec<Usage> = self
            .workflows
            .iter()
            .flat_map(|workflow| {
                workflow
                    .steps
                    .iter()
                    .enumerate()
                    .filter(|(_, step)| step.action == id)
                    .map(|(index, _)| Usage::WorkflowStep {
                        workflow: workflow.id,
                        workflow_name: workflow.name.clone(),
                        position: index + 1,
                    })
            })
            .collect();
        usages.extend(self.target_usages(|t| *t == BindingTarget::Action { action: id }));
        usages
    }

    /// Layout bindings that run a workflow.
    pub fn workflow_usages(&self, id: WorkflowId) -> Vec<Usage> {
        self.target_usages(|t| *t == BindingTarget::Workflow { workflow: id })
    }

    /// Deletes an action that nothing refers to.
    pub fn delete_action(&mut self, id: ActionInstanceId) -> Result<ActionInstance, ProfileIssue> {
        let used = self.action_usages(id).len();
        if used > 0 {
            return Err(ProfileIssue::InUse(used));
        }
        let index = self
            .actions
            .iter()
            .position(|action| action.id == id)
            .ok_or(ProfileIssue::NotFound("action"))?;
        Ok(self.actions.remove(index))
    }

    /// Deletes a workflow that no layout refers to.
    pub fn delete_workflow(&mut self, id: WorkflowId) -> Result<Workflow, ProfileIssue> {
        let used = self.workflow_usages(id).len();
        if used > 0 {
            return Err(ProfileIssue::InUse(used));
        }
        let index = self
            .workflows
            .iter()
            .position(|workflow| workflow.id == id)
            .ok_or(ProfileIssue::NotFound("workflow"))?;
        Ok(self.workflows.remove(index))
    }

    /// The layout used for `model`: the assigned one, otherwise the first.
    pub fn layout_for(&self, model: &DeviceModelId) -> Option<&DeviceLayout> {
        self.active_layouts
            .get(model)
            .and_then(|id| self.layout(*id))
            .filter(|layout| &layout.model == model)
            .or_else(|| self.layouts.iter().find(|layout| &layout.model == model))
    }

    /// Makes `layout` the one used for its device model.
    pub fn activate_layout(&mut self, id: LayoutId) -> Result<(), ProfileIssue> {
        let model = self
            .layout(id)
            .ok_or(ProfileIssue::NotFound("layout"))?
            .model
            .clone();
        self.active_layouts.insert(model, id);
        Ok(())
    }

    /// Deletes a layout and its assignment.
    pub fn delete_layout(&mut self, id: LayoutId) -> Result<DeviceLayout, ProfileIssue> {
        let index = self
            .layouts
            .iter()
            .position(|layout| layout.id == id)
            .ok_or(ProfileIssue::NotFound("layout"))?;
        self.active_layouts.retain(|_, layout| *layout != id);
        Ok(self.layouts.remove(index))
    }

    /// Copies a layout with new layout and page identifiers.
    pub fn duplicate_layout(&mut self, id: LayoutId, name: &str) -> Result<LayoutId, ProfileIssue> {
        let source = self.layout(id).ok_or(ProfileIssue::NotFound("layout"))?;
        let mut copy = source.clone();
        let pages: HashMap<PageId, PageId> = copy
            .pages
            .iter()
            .map(|page| (page.id, PageId::new()))
            .collect();
        remap_layout(
            &mut copy,
            LayoutId::new(),
            &pages,
            &HashMap::new(),
            &HashMap::new(),
        );
        name.trim().clone_into(&mut copy.name);
        let new_id = copy.id;
        self.layouts.push(copy);
        Ok(new_id)
    }

    /// An independent copy with new identifiers for the profile and
    /// everything in it. References inside the copy point to the copy.
    #[must_use]
    pub fn duplicate(&self, name: &str) -> Profile {
        let mut copy = self.clone();
        copy.id = ProfileId::new();
        name.trim().clone_into(&mut copy.name);

        let projects: HashMap<ProjectId, ProjectId> = copy
            .projects
            .iter()
            .map(|p| (p.id, ProjectId::new()))
            .collect();
        let actions: HashMap<ActionInstanceId, ActionInstanceId> = copy
            .actions
            .iter()
            .map(|a| (a.id, ActionInstanceId::new()))
            .collect();
        let workflows: HashMap<WorkflowId, WorkflowId> = copy
            .workflows
            .iter()
            .map(|w| (w.id, WorkflowId::new()))
            .collect();
        let layouts: HashMap<LayoutId, LayoutId> = copy
            .layouts
            .iter()
            .map(|l| (l.id, LayoutId::new()))
            .collect();

        for project in &mut copy.projects {
            project.id = projects[&project.id];
        }
        copy.default_project = copy
            .default_project
            .and_then(|id| projects.get(&id).copied());
        for action in &mut copy.actions {
            action.id = actions[&action.id];
        }
        for workflow in &mut copy.workflows {
            workflow.id = workflows[&workflow.id];
            for step in &mut workflow.steps {
                if let Some(id) = actions.get(&step.action) {
                    step.action = *id;
                }
                if let StepCondition::ActiveProject { project } = &mut step.condition
                    && let Some(id) = projects.get(project)
                {
                    *project = *id;
                }
            }
        }
        for layout in &mut copy.layouts {
            let pages: HashMap<PageId, PageId> = layout
                .pages
                .iter()
                .map(|page| (page.id, PageId::new()))
                .collect();
            let id = layouts[&layout.id];
            remap_layout(layout, id, &pages, &actions, &workflows);
        }
        copy.active_layouts = copy
            .active_layouts
            .into_iter()
            .filter_map(|(model, layout)| layouts.get(&layout).map(|id| (model, *id)))
            .collect();
        copy
    }

    /// Checks internal consistency. Layouts are validated against `catalog`.
    pub fn validate(&self, catalog: &ModelCatalog) -> Vec<ProfileIssue> {
        let mut issues = Vec::new();
        if self.name.trim().is_empty() {
            issues.push(ProfileIssue::EmptyName);
        }
        for (index, project) in self.projects.iter().enumerate() {
            let duplicate = self.projects[..index]
                .iter()
                .any(|p| p.key.matches(project.key.as_str()));
            if duplicate {
                issues.push(ProfileIssue::DuplicateProjectKey(project.key.to_string()));
            }
        }
        for workflow in &self.workflows {
            for step in &workflow.steps {
                if self.action(step.action).is_none() {
                    issues.push(ProfileIssue::MissingWorkflowAction {
                        workflow: workflow.name.clone(),
                        action: step.action,
                    });
                }
            }
        }
        for layout in &self.layouts {
            let found = layout.validate(catalog, self);
            issues.extend(found.into_iter().map(|issue| ProfileIssue::Layout {
                layout: layout.name.clone(),
                issue,
            }));
        }
        issues
    }
}

fn remap_layout(
    layout: &mut DeviceLayout,
    id: LayoutId,
    pages: &HashMap<PageId, PageId>,
    actions: &HashMap<ActionInstanceId, ActionInstanceId>,
    workflows: &HashMap<WorkflowId, WorkflowId>,
) {
    layout.id = id;
    if let Some(home) = pages.get(&layout.home_page) {
        layout.home_page = *home;
    }
    for page in &mut layout.pages {
        if let Some(new) = pages.get(&page.id) {
            page.id = *new;
        }
        for binding in &mut page.bindings {
            match &mut binding.target {
                BindingTarget::Action { action } => {
                    if let Some(new) = actions.get(action) {
                        *action = *new;
                    }
                }
                BindingTarget::Workflow { workflow } => {
                    if let Some(new) = workflows.get(workflow) {
                        *workflow = *new;
                    }
                }
                BindingTarget::OpenPage { page } => {
                    if let Some(new) = pages.get(page) {
                        *page = *new;
                    }
                }
                BindingTarget::Back => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionParameters, OnFailure, ProjectKey, builtin};

    fn action(name: &str) -> ActionInstance {
        ActionInstance {
            id: ActionInstanceId::new(),
            name: name.into(),
            action_type: builtin::log_message_id(),
            parameters: ActionParameters::new(),
            description: String::new(),
            icon: None,
            label: None,
        }
    }

    #[test]
    fn same_workflow_name_differs_between_profiles() {
        let mut work = Profile::new("Work");
        let mut home = Profile::new("Home");
        let a = action("Open IDE");
        let b = action("Play music");
        work.workflows
            .push(Workflow::new("Start Work").step(a.id, OnFailure::Stop));
        home.workflows
            .push(Workflow::new("Start Work").step(b.id, OnFailure::Stop));
        work.actions.push(a);
        home.actions.push(b);

        assert_ne!(work.workflows[0].steps, home.workflows[0].steps);
        assert_eq!(
            work.validate(&ModelCatalog::builtin()),
            Vec::<ProfileIssue>::new()
        );
        assert_eq!(
            home.validate(&ModelCatalog::builtin()),
            Vec::<ProfileIssue>::new()
        );
    }

    #[test]
    fn rejects_duplicate_project_keys() {
        let mut profile = Profile::new("Work");
        profile
            .add_project(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"))
            .unwrap();
        let duplicate = Project::new(ProjectKey::new("shop").unwrap(), "Shop 2");
        assert!(profile.add_project(duplicate.clone()).is_err());

        profile.projects.push(duplicate);
        let issues = profile.validate(&ModelCatalog::builtin());
        assert_eq!(
            issues,
            vec![ProfileIssue::DuplicateProjectKey("shop".into())]
        );
    }

    #[test]
    fn referenced_actions_cannot_be_deleted() {
        let mut profile = Profile::new("Work");
        let a = action("A");
        let id = a.id;
        profile.actions.push(a);
        profile
            .workflows
            .push(Workflow::new("W").step(id, OnFailure::Stop));
        let usages = profile.action_usages(id);
        assert_eq!(usages.len(), 1);
        assert!(matches!(usages[0], Usage::WorkflowStep { position: 1, .. }));
        assert_eq!(profile.delete_action(id), Err(ProfileIssue::InUse(1)));

        profile.workflows[0].steps.clear();
        assert!(profile.delete_action(id).is_ok());
        assert_eq!(
            profile.delete_action(id),
            Err(ProfileIssue::NotFound("action"))
        );
    }

    #[test]
    fn duplicated_profiles_are_independent() {
        use crate::{ControlBinding, Trigger};
        use deckpilot_device::models;

        let mut profile = Profile::new("Work");
        let a = action("Open IDE");
        let a_id = a.id;
        profile.actions.push(a);
        profile
            .add_project(Project::new(
                crate::ProjectKey::new("SHOP").unwrap(),
                "Shop",
            ))
            .unwrap();
        let shop = profile.projects[0].id;
        profile.default_project = Some(shop);
        let mut workflow = Workflow::new("Start Work").step(a_id, OnFailure::Stop);
        workflow.steps[0].condition = StepCondition::ActiveProject { project: shop };
        let w_id = workflow.id;
        profile.workflows.push(workflow);
        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        let home = layout.home_page;
        layout.pages[0].bindings.push(ControlBinding {
            control: ControlId::Key { index: 0 },
            trigger: Trigger::Press,
            target: BindingTarget::Workflow { workflow: w_id },
            label: None,
        });
        let l_id = layout.id;
        profile.layouts.push(layout);
        profile.activate_layout(l_id).unwrap();

        let mut copy = profile.duplicate(" Home ");
        assert_eq!(copy.name, "Home");
        assert_ne!(copy.id, profile.id);
        assert_ne!(copy.actions[0].id, a_id);
        assert_ne!(copy.workflows[0].id, w_id);
        assert_ne!(copy.layouts[0].id, l_id);
        assert_ne!(copy.layouts[0].home_page, home);
        // References inside the copy point to the copy.
        assert_eq!(copy.workflows[0].steps[0].action, copy.actions[0].id);
        assert_eq!(
            copy.workflows[0].steps[0].condition,
            StepCondition::ActiveProject {
                project: copy.projects[0].id
            }
        );
        assert_eq!(copy.default_project, Some(copy.projects[0].id));
        assert_eq!(
            copy.layouts[0].pages[0].bindings[0].target,
            BindingTarget::Workflow {
                workflow: copy.workflows[0].id
            }
        );
        assert_eq!(
            copy.layout_for(&copy.layouts[0].model).unwrap().id,
            copy.layouts[0].id
        );
        assert_eq!(copy.validate(&ModelCatalog::builtin()), Vec::new());

        // Editing the copy leaves the original untouched.
        copy.workflows[0].steps.clear();
        assert_eq!(profile.workflows[0].steps.len(), 1);
    }

    #[test]
    fn projects_can_be_reordered_updated_and_deleted() {
        let mut profile = Profile::new("Work");
        for key in ["SHOP", "CMR", "CMS"] {
            profile
                .add_project(Project::new(crate::ProjectKey::new(key).unwrap(), key))
                .unwrap();
        }
        let cms = profile.projects[2].id;
        profile.move_project(cms, 0).unwrap();
        let keys: Vec<_> = profile.projects.iter().map(|p| p.key.to_string()).collect();
        assert_eq!(keys, ["CMS", "SHOP", "CMR"]);

        let mut renamed = profile.projects[0].clone();
        renamed.key = crate::ProjectKey::new("shop").unwrap();
        assert!(matches!(
            profile.update_project(renamed),
            Err(ProfileIssue::DuplicateProjectKey(_))
        ));

        profile.default_project = Some(cms);
        profile.delete_project(cms).unwrap();
        assert_eq!(profile.default_project, None);
        assert_eq!(profile.projects.len(), 2);
    }

    #[test]
    fn layouts_can_be_assigned_duplicated_and_deleted() {
        use deckpilot_device::models;
        let model = DeviceModelId::new(models::STREAM_DECK_MK2_ID);
        let mut profile = Profile::new("Work");
        let first = DeviceLayout::new("First", model.clone());
        let second = DeviceLayout::new("Second", model.clone());
        let (first_id, second_id) = (first.id, second.id);
        profile.layouts.extend([first, second]);
        assert_eq!(profile.layout_for(&model).unwrap().id, first_id);
        profile.activate_layout(second_id).unwrap();
        assert_eq!(profile.layout_for(&model).unwrap().id, second_id);

        let copy = profile.duplicate_layout(second_id, "Copy").unwrap();
        assert_ne!(copy, second_id);
        assert_eq!(profile.layouts.len(), 3);

        profile.delete_layout(second_id).unwrap();
        assert_eq!(profile.layout_for(&model).unwrap().id, first_id);
    }

    #[test]
    fn detects_dangling_workflow_steps() {
        let mut profile = Profile::new("Work");
        profile
            .workflows
            .push(Workflow::new("W").step(ActionInstanceId::new(), OnFailure::Stop));
        let issues = profile.validate(&ModelCatalog::builtin());
        assert!(matches!(
            issues[0],
            ProfileIssue::MissingWorkflowAction { .. }
        ));
    }

    #[test]
    fn profiles_round_trip_through_json() {
        let mut profile = Profile::new("Work");
        profile.actions.push(action("A"));
        profile
            .projects
            .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
        let json = serde_json::to_string(&profile).unwrap();
        assert_eq!(serde_json::from_str::<Profile>(&json).unwrap(), profile);
    }
}
