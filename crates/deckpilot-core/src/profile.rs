//! Profiles: the portable unit of user configuration.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use deckpilot_device::ModelCatalog;

use crate::{
    ActionInstance, ActionInstanceId, DeviceLayout, LayoutIssue, ProfileId, Project, ProjectId,
    Workflow, WorkflowId,
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
    #[serde(default)]
    pub layouts: Vec<DeviceLayout>,
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
            layouts: Vec::new(),
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

    /// Removes an action and every workflow step that referenced it.
    pub fn remove_action(&mut self, id: ActionInstanceId) -> Option<ActionInstance> {
        let index = self.actions.iter().position(|action| action.id == id)?;
        for workflow in &mut self.workflows {
            workflow.steps.retain(|step| step.action != id);
        }
        Some(self.actions.remove(index))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionParameters, OnFailure, ProjectKey, WorkflowStep, builtin};

    fn action(name: &str) -> ActionInstance {
        ActionInstance {
            id: ActionInstanceId::new(),
            name: name.into(),
            action_type: builtin::log_message_id(),
            parameters: ActionParameters::new(),
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
    fn removing_an_action_cleans_up_workflows() {
        let mut profile = Profile::new("Work");
        let a = action("A");
        let id = a.id;
        profile.actions.push(a);
        profile
            .workflows
            .push(Workflow::new("W").step(id, OnFailure::Stop));
        assert!(profile.remove_action(id).is_some());
        assert_eq!(profile.workflows[0].steps, Vec::<WorkflowStep>::new());
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
