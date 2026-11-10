//! Managing the projects of a profile.
//!
//! Every operation works on exactly one profile. Projects are never shared:
//! [`ProjectCatalog::copy_project_to`] is the only way to bring a project into
//! another profile, and it creates an independent copy with a new id.

use deckpilot_core::{Profile, Project, ProjectId};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CatalogError {
    #[error("project key `{0}` is already used in this profile")]
    DuplicateKey(String),
    #[error("project does not exist in this profile")]
    NotFound,
    #[error("the project is still used in {0} place(s); remove those references first")]
    InUse(usize),
    #[error("a project id may only be used once across all profiles")]
    IdInUse,
    #[error("source and target are the same profile")]
    SameProfile,
}

/// Project management for a [`Profile`].
pub trait ProjectCatalog {
    /// Adds a project at the end of the order unless its key is taken.
    fn add_project(&mut self, project: Project) -> Result<ProjectId, CatalogError>;
    /// Replaces a project's key, name and resources, keeping its position.
    fn update_project(&mut self, project: Project) -> Result<(), CatalogError>;
    /// Moves a project to `index` in the order (clamped to the end).
    fn move_project(&mut self, id: ProjectId, index: usize) -> Result<(), CatalogError>;
    /// Removes a project that nothing refers to.
    fn delete_project(&mut self, id: ProjectId) -> Result<Project, CatalogError>;
    /// Sets (or clears) the project selected when the profile is activated.
    fn set_default_project(&mut self, id: Option<ProjectId>) -> Result<(), CatalogError>;
    /// Copies a project into `target` with a new id. Fails if the key is
    /// already used there.
    fn copy_project_to(
        &self,
        id: ProjectId,
        target: &mut Profile,
    ) -> Result<ProjectId, CatalogError>;
}

fn key_taken(profile: &Profile, key: &str, except: Option<ProjectId>) -> bool {
    profile
        .projects
        .iter()
        .any(|p| Some(p.id) != except && p.key.matches(key))
}

impl ProjectCatalog for Profile {
    fn add_project(&mut self, project: Project) -> Result<ProjectId, CatalogError> {
        if key_taken(self, project.key.as_str(), None) {
            return Err(CatalogError::DuplicateKey(project.key.to_string()));
        }
        if self.project(project.id).is_some() {
            return Err(CatalogError::IdInUse);
        }
        let id = project.id;
        self.projects.push(project);
        Ok(id)
    }

    fn update_project(&mut self, project: Project) -> Result<(), CatalogError> {
        if key_taken(self, project.key.as_str(), Some(project.id)) {
            return Err(CatalogError::DuplicateKey(project.key.to_string()));
        }
        let slot = self
            .projects
            .iter_mut()
            .find(|p| p.id == project.id)
            .ok_or(CatalogError::NotFound)?;
        *slot = project;
        Ok(())
    }

    fn move_project(&mut self, id: ProjectId, index: usize) -> Result<(), CatalogError> {
        let from = self
            .projects
            .iter()
            .position(|p| p.id == id)
            .ok_or(CatalogError::NotFound)?;
        let project = self.projects.remove(from);
        let to = index.min(self.projects.len());
        self.projects.insert(to, project);
        Ok(())
    }

    fn delete_project(&mut self, id: ProjectId) -> Result<Project, CatalogError> {
        let index = self
            .projects
            .iter()
            .position(|p| p.id == id)
            .ok_or(CatalogError::NotFound)?;
        let references = self.project_references(id);
        if references > 0 {
            return Err(CatalogError::InUse(references));
        }
        if self.default_project == Some(id) {
            self.default_project = None;
        }
        Ok(self.projects.remove(index))
    }

    fn set_default_project(&mut self, id: Option<ProjectId>) -> Result<(), CatalogError> {
        if let Some(id) = id
            && self.project(id).is_none()
        {
            return Err(CatalogError::NotFound);
        }
        self.default_project = id;
        Ok(())
    }

    fn copy_project_to(
        &self,
        id: ProjectId,
        target: &mut Profile,
    ) -> Result<ProjectId, CatalogError> {
        if self.id == target.id {
            return Err(CatalogError::SameProfile);
        }
        let source = self.project(id).ok_or(CatalogError::NotFound)?;
        let mut copy = Project::new(source.key.clone(), source.name.clone());
        copy.resources.clone_from(&source.resources);
        target.add_project(copy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::{
        ActionInstance, ActionInstanceId, ActionParameters, OnFailure, ParameterValue, ProjectKey,
        ResourceKey, StepCondition, Workflow,
    };

    fn project(key: &str) -> Project {
        Project::new(ProjectKey::new(key).unwrap(), key)
    }

    fn keys(profile: &Profile) -> Vec<String> {
        profile.projects.iter().map(|p| p.key.to_string()).collect()
    }

    fn work() -> Profile {
        let mut profile = Profile::new("Work");
        for key in ["SHOP", "CMR", "CMS"] {
            profile.add_project(project(key)).unwrap();
        }
        profile
    }

    #[test]
    fn projects_are_ordered_and_keys_unique_per_profile() {
        let mut profile = work();
        let cms = profile.projects[2].id;
        profile.move_project(cms, 0).unwrap();
        assert_eq!(keys(&profile), ["CMS", "SHOP", "CMR"]);
        profile.move_project(cms, 99).unwrap();
        assert_eq!(keys(&profile), ["SHOP", "CMR", "CMS"]);
        assert_eq!(
            profile.move_project(ProjectId::new(), 0),
            Err(CatalogError::NotFound)
        );

        assert_eq!(
            profile.add_project(project("shop")),
            Err(CatalogError::DuplicateKey("shop".into()))
        );
        let mut renamed = profile.projects[0].clone();
        renamed.key = ProjectKey::new("cmr").unwrap();
        assert!(matches!(
            profile.update_project(renamed),
            Err(CatalogError::DuplicateKey(_))
        ));

        // The same key is fine in a different profile.
        let mut home = Profile::new("Home");
        home.add_project(project("SHOP")).unwrap();
    }

    #[test]
    fn referenced_projects_cannot_be_deleted() {
        let mut profile = work();
        let shop = profile.projects[0].id;
        let action = ActionInstance {
            id: ActionInstanceId::new(),
            name: "Select".into(),
            action_type: crate::actions::select_id(),
            parameters: ActionParameters::new().with("project_id", ParameterValue::Project(shop)),
            description: String::new(),
            icon: None,
            label: None,
        };
        let action_id = action.id;
        profile.actions.push(action);
        let mut workflow = Workflow::new("W").step(action_id, OnFailure::Stop);
        workflow.steps[0].condition = StepCondition::ActiveProject { project: shop };
        profile.workflows.push(workflow);
        assert_eq!(profile.delete_project(shop), Err(CatalogError::InUse(2)));

        profile.workflows.clear();
        profile.actions.clear();
        profile.set_default_project(Some(shop)).unwrap();
        profile.delete_project(shop).unwrap();
        assert_eq!(profile.default_project, None);
        assert_eq!(keys(&profile), ["CMR", "CMS"]);
    }

    #[test]
    fn default_project_must_belong_to_the_profile() {
        let mut profile = work();
        let other = work();
        assert_eq!(
            profile.set_default_project(Some(other.projects[0].id)),
            Err(CatalogError::NotFound)
        );
        profile
            .set_default_project(Some(profile.projects[1].id))
            .unwrap();
        profile.set_default_project(None).unwrap();
    }

    #[test]
    fn copying_creates_an_independent_project() {
        let mut work = work();
        work.projects[0]
            .resources
            .insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
        let shop = work.projects[0].id;
        let mut home = Profile::new("Home");

        let copy = work.copy_project_to(shop, &mut home).unwrap();
        assert_ne!(copy, shop);
        assert_eq!(keys(&home), ["SHOP"]);
        assert_eq!(home.projects[0].resources, work.projects[0].resources);

        // Changing the copy leaves the original untouched.
        home.projects[0].name = "Home shop".into();
        assert_eq!(work.projects[0].name, "SHOP");

        assert_eq!(
            work.copy_project_to(shop, &mut home),
            Err(CatalogError::DuplicateKey("SHOP".into()))
        );
        let mut same = work.clone();
        assert_eq!(
            work.copy_project_to(shop, &mut same),
            Err(CatalogError::SameProfile)
        );
    }
}
