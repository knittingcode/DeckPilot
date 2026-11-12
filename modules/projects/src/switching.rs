//! Switching the active project of a profile.
//!
//! Used by the agent and the configuration app when the user switches from
//! the tray or the window. The `projects.cycle`, `projects.next` and
//! `projects.previous` actions request the same step as a
//! [`ContextEffect::CycleProject`](deckpilot_core::ContextEffect), which the
//! engine applies with [`ProjectSwitcher::step`], so every path follows the
//! same order and the module's `wrapAround` setting.

use deckpilot_core::{CycleDirection, ModuleRegistry, Profile, ProjectId, ProjectSwitcher};

use crate::{WRAP_AROUND, module_id};

pub use deckpilot_core::ProjectStepError as StepError;

/// The project after (or before) `current` in the profile's order. Without
/// wrap-around, stepping past either end is an error. A missing or stale
/// `current` starts at the first (or last) project.
pub fn step(
    profile: &Profile,
    current: Option<ProjectId>,
    direction: CycleDirection,
    wrap_around: bool,
) -> Result<ProjectId, StepError> {
    ProjectSwitcher::step(&profile.projects, current, direction, wrap_around)
}

/// The module's `wrapAround` setting as currently configured in `modules`.
pub fn wrap_around(modules: &ModuleRegistry) -> bool {
    modules
        .states()
        .get(&module_id())
        .and_then(|settings| settings.config.boolean(WRAP_AROUND))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::{ActionParameters, Os, ParameterValue, Project, ProjectKey};

    fn profile() -> Profile {
        let mut profile = Profile::new("Work");
        for key in ["A", "B"] {
            profile
                .projects
                .push(Project::new(ProjectKey::new(key).unwrap(), key));
        }
        profile
    }

    #[test]
    fn steps_follow_the_order_and_the_wrap_setting() {
        let profile = profile();
        let (a, b) = (profile.projects[0].id, profile.projects[1].id);
        assert_eq!(step(&profile, Some(a), CycleDirection::Next, true), Ok(b));
        assert_eq!(step(&profile, Some(b), CycleDirection::Next, true), Ok(a));
        assert_eq!(
            step(&profile, Some(b), CycleDirection::Next, false),
            Err(StepError::AtLast)
        );
        assert_eq!(
            step(&profile, Some(a), CycleDirection::Previous, false),
            Err(StepError::AtFirst)
        );
        assert_eq!(step(&profile, None, CycleDirection::Previous, false), Ok(b));
        assert_eq!(
            step(&Profile::new("Empty"), None, CycleDirection::Next, true),
            Err(StepError::NoProjects)
        );
    }

    #[test]
    fn wrap_setting_is_read_from_the_registry() {
        let mut modules = ModuleRegistry::new(Os::Linux);
        modules
            .register(Box::new(crate::ProjectsModule::new()))
            .unwrap();
        assert!(wrap_around(&modules));
        modules
            .set_config(
                &module_id(),
                &ActionParameters::new().with(WRAP_AROUND, ParameterValue::Boolean(false)),
            )
            .unwrap();
        assert!(!wrap_around(&modules));
    }
}
