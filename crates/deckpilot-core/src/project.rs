//! Projects and the active project context.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{IdentifierError, Profile, ProfileId, ProjectId, ResourceKey};

/// Short, human-facing project code such as `SHOP`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectKey(String);

impl ProjectKey {
    pub fn new(key: impl Into<String>) -> Result<Self, IdentifierError> {
        let key = key.into();
        let error = |reason| IdentifierError {
            kind: "project key",
            value: key.clone(),
            reason,
        };
        if key.is_empty() || key.len() > 32 {
            return Err(error("must be between 1 and 32 characters"));
        }
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            return Err(error("only ASCII letters, digits, `_` and `-` are allowed"));
        }
        Ok(Self(key))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Project keys are compared case-insensitively for uniqueness.
    pub fn matches(&self, other: &str) -> bool {
        self.0.eq_ignore_ascii_case(other)
    }
}

impl TryFrom<String> for ProjectKey {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ProjectKey> for String {
    fn from(value: ProjectKey) -> Self {
        value.0
    }
}

impl fmt::Display for ProjectKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A project inside a profile.
///
/// `resources` maps a *role* (e.g. `repository`, `compose`) to a logical
/// resource key. Project-dependent actions refer to roles, so the same action
/// targets a different resource depending on the active project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: ProjectId,
    pub key: ProjectKey,
    pub name: String,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceKey>,
}

impl Project {
    pub fn new(key: ProjectKey, name: impl Into<String>) -> Self {
        Self {
            id: ProjectId::new(),
            key,
            name: name.into(),
            resources: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CycleDirection {
    Next,
    Previous,
}

/// Cyclic selection over an ordered list of projects: `A -> B -> C -> A`.
#[derive(Debug, Clone, Copy)]
pub struct ProjectSwitcher;

impl ProjectSwitcher {
    /// Returns the project that follows `current` in `direction`, wrapping
    /// around at either end. When `current` is `None` or no longer exists,
    /// selection starts at the first (or last) project.
    pub fn cycle(
        projects: &[Project],
        current: Option<ProjectId>,
        direction: CycleDirection,
    ) -> Option<ProjectId> {
        let len = projects.len();
        if len == 0 {
            return None;
        }
        let position = current.and_then(|id| projects.iter().position(|p| p.id == id));
        let index = match (position, direction) {
            (Some(i), CycleDirection::Next) => (i + 1) % len,
            (Some(i), CycleDirection::Previous) => (i + len - 1) % len,
            (None, CycleDirection::Next) => 0,
            (None, CycleDirection::Previous) => len - 1,
        };
        Some(projects[index].id)
    }

    /// Like [`cycle`](Self::cycle), but without `wrap_around` stepping past
    /// either end is an error. A missing or stale `current` (e.g. a deleted
    /// project) starts at the first (or last) project, so it never blocks
    /// switching. The order is the profile's current order.
    pub fn step(
        projects: &[Project],
        current: Option<ProjectId>,
        direction: CycleDirection,
        wrap_around: bool,
    ) -> Result<ProjectId, ProjectStepError> {
        if projects.is_empty() {
            return Err(ProjectStepError::NoProjects);
        }
        if !wrap_around
            && let Some(position) = current.and_then(|id| projects.iter().position(|p| p.id == id))
        {
            match direction {
                CycleDirection::Next if position + 1 == projects.len() => {
                    return Err(ProjectStepError::AtLast);
                }
                CycleDirection::Previous if position == 0 => return Err(ProjectStepError::AtFirst),
                _ => {}
            }
        }
        Self::cycle(projects, current, direction).ok_or(ProjectStepError::NoProjects)
    }
}

/// Why stepping through a profile's projects selected nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProjectStepError {
    #[error("this profile has no projects")]
    NoProjects,
    #[error("already at the last project")]
    AtLast,
    #[error("already at the first project")]
    AtFirst,
}

impl ProjectStepError {
    /// Stable error code for [`ActionResult::code`](crate::ActionResult::code).
    pub fn code(self) -> &'static str {
        match self {
            Self::NoProjects => "no-projects",
            Self::AtLast => "at-last-project",
            Self::AtFirst => "at-first-project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectContextError {
    #[error("project context belongs to a different profile")]
    ProfileMismatch,
    #[error("project does not exist in this profile")]
    UnknownProject,
}

/// The active project of one profile.
///
/// The context is bound to a single profile; using it with another profile is
/// an error. Switching only changes which project is considered active; it
/// never touches Git branches, containers, or running applications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveProjectContext {
    profile_id: ProfileId,
    project_id: Option<ProjectId>,
}

impl ActiveProjectContext {
    /// Creates a context for `profile`, selecting its default project, or the
    /// first project if no valid default is set.
    pub fn new(profile: &Profile) -> Self {
        let default = profile
            .default_project
            .filter(|id| profile.project(*id).is_some());
        Self {
            profile_id: profile.id,
            project_id: default.or_else(|| profile.projects.first().map(|p| p.id)),
        }
    }

    /// Restores a previously persisted selection. Stale ids fall back to the
    /// default (or first) project of the profile.
    pub fn restore(profile: &Profile, project_id: Option<ProjectId>) -> Self {
        let mut context = Self::new(profile);
        if let Some(id) = project_id.filter(|id| profile.project(*id).is_some()) {
            context.project_id = Some(id);
        }
        context
    }

    pub fn profile_id(&self) -> ProfileId {
        self.profile_id
    }

    pub fn project_id(&self) -> Option<ProjectId> {
        self.project_id
    }

    /// The active project, looked up in `profile`.
    pub fn project<'a>(
        &self,
        profile: &'a Profile,
    ) -> Result<Option<&'a Project>, ProjectContextError> {
        self.check(profile)?;
        Ok(self.project_id.and_then(|id| profile.project(id)))
    }

    pub fn select(
        &mut self,
        profile: &Profile,
        project: ProjectId,
    ) -> Result<(), ProjectContextError> {
        self.check(profile)?;
        if profile.project(project).is_none() {
            return Err(ProjectContextError::UnknownProject);
        }
        self.project_id = Some(project);
        Ok(())
    }

    pub fn cycle(
        &mut self,
        profile: &Profile,
        direction: CycleDirection,
    ) -> Result<Option<ProjectId>, ProjectContextError> {
        self.check(profile)?;
        self.project_id = ProjectSwitcher::cycle(&profile.projects, self.project_id, direction);
        Ok(self.project_id)
    }

    fn check(&self, profile: &Profile) -> Result<(), ProjectContextError> {
        if profile.id == self.profile_id {
            Ok(())
        } else {
            Err(ProjectContextError::ProfileMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_with(keys: &[&str]) -> Profile {
        let mut profile = Profile::new("Work");
        for key in keys {
            profile
                .projects
                .push(Project::new(ProjectKey::new(*key).unwrap(), *key));
        }
        profile
    }

    fn key_of(profile: &Profile, id: Option<ProjectId>) -> &str {
        profile.project(id.unwrap()).unwrap().key.as_str()
    }

    #[test]
    fn project_keys_are_validated() {
        assert!(ProjectKey::new("SHOP").is_ok());
        assert!(ProjectKey::new("cms-2").is_ok());
        assert!(ProjectKey::new("").is_err());
        assert!(ProjectKey::new("with space").is_err());
        assert!(ProjectKey::new("x".repeat(33)).is_err());
        assert!(ProjectKey::new("Shop").unwrap().matches("SHOP"));
    }

    #[test]
    fn cycles_forward_with_wraparound() {
        let profile = profile_with(&["SHOP", "CMR", "CMS"]);
        let mut context = ActiveProjectContext::new(&profile);
        assert_eq!(key_of(&profile, context.project_id()), "SHOP");

        let mut visited = Vec::new();
        for _ in 0..4 {
            let id = context.cycle(&profile, CycleDirection::Next).unwrap();
            visited.push(key_of(&profile, id).to_owned());
        }
        assert_eq!(visited, ["CMR", "CMS", "SHOP", "CMR"]);
    }

    #[test]
    fn cycles_backward_with_wraparound() {
        let profile = profile_with(&["SHOP", "CMR", "CMS"]);
        let mut context = ActiveProjectContext::new(&profile);
        let id = context.cycle(&profile, CycleDirection::Previous).unwrap();
        assert_eq!(key_of(&profile, id), "CMS");
    }

    #[test]
    fn cycle_handles_empty_single_and_unknown() {
        assert_eq!(
            ProjectSwitcher::cycle(&[], None, CycleDirection::Next),
            None
        );

        let single = profile_with(&["ONE"]);
        let only = single.projects[0].id;
        assert_eq!(
            ProjectSwitcher::cycle(&single.projects, Some(only), CycleDirection::Next),
            Some(only)
        );

        let profile = profile_with(&["A", "B"]);
        let stale = Some(ProjectId::new());
        assert_eq!(
            ProjectSwitcher::cycle(&profile.projects, stale, CycleDirection::Next),
            Some(profile.projects[0].id)
        );
    }

    #[test]
    fn step_follows_the_order_and_the_wrap_setting() {
        let profile = profile_with(&["A", "B", "C"]);
        let ids: Vec<_> = profile.projects.iter().map(|p| p.id).collect();
        let step = |current, direction, wrap| {
            ProjectSwitcher::step(&profile.projects, current, direction, wrap)
        };
        assert_eq!(step(Some(ids[2]), CycleDirection::Next, true), Ok(ids[0]));
        assert_eq!(
            step(Some(ids[0]), CycleDirection::Previous, true),
            Ok(ids[2])
        );
        assert_eq!(
            step(Some(ids[2]), CycleDirection::Next, false),
            Err(ProjectStepError::AtLast)
        );
        assert_eq!(
            step(Some(ids[0]), CycleDirection::Previous, false),
            Err(ProjectStepError::AtFirst)
        );
        // A deleted (stale) or missing current project starts at an end,
        // even without wrap-around.
        let stale = Some(ProjectId::new());
        assert_eq!(step(stale, CycleDirection::Next, false), Ok(ids[0]));
        assert_eq!(step(None, CycleDirection::Previous, false), Ok(ids[2]));
        assert_eq!(
            ProjectSwitcher::step(&[], None, CycleDirection::Next, true),
            Err(ProjectStepError::NoProjects)
        );
        assert_eq!(ProjectStepError::NoProjects.code(), "no-projects");
    }

    #[test]
    fn context_is_scoped_to_its_profile() {
        let work = profile_with(&["SHOP", "CMR"]);
        let home = profile_with(&["BLOG"]);
        let mut context = ActiveProjectContext::new(&work);

        assert_eq!(
            context.cycle(&home, CycleDirection::Next),
            Err(ProjectContextError::ProfileMismatch)
        );
        assert_eq!(
            context.select(&work, home.projects[0].id),
            Err(ProjectContextError::UnknownProject)
        );
        assert!(context.project(&home).is_err());
        assert_eq!(
            context.project(&work).unwrap().unwrap().key.as_str(),
            "SHOP"
        );
    }

    #[test]
    fn restore_ignores_stale_selection() {
        let profile = profile_with(&["A", "B"]);
        let second = profile.projects[1].id;
        assert_eq!(
            ActiveProjectContext::restore(&profile, Some(second)).project_id(),
            Some(second)
        );
        assert_eq!(
            ActiveProjectContext::restore(&profile, Some(ProjectId::new())).project_id(),
            Some(profile.projects[0].id)
        );
    }
}
