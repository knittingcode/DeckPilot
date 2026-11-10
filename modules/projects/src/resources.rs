//! The one resolver behind every project resource action.
//!
//! `projects.repository` and `projects.documentation` are the same resolver
//! with a fixed resource name; `projects.resolve-resource` takes the name as
//! a parameter. The resolver only reports what is bound on this machine.
//! Opening a folder, a browser or running Git belongs to the modules for
//! those tools.

use deckpilot_core::{ActionParameters, ActionResult, ExecutionContext, Project, ResourceKind};

use crate::actions::PROJECT_ID;

/// Resource name used by `projects.repository`.
pub const REPOSITORY_ROLE: &str = "repository";
/// Resource name used by `projects.documentation`.
pub const DOCUMENTATION_ROLE: &str = "docs";

/// The project an action refers to: its `project_id` parameter, or the
/// active project when unset. An explicit project must belong to the active
/// profile.
pub(crate) fn target_project<'a>(
    parameters: &ActionParameters,
    context: &ExecutionContext<'a>,
) -> Result<&'a Project, String> {
    match parameters.project(PROJECT_ID) {
        Some(id) => context
            .profile()
            .map_err(|denied| denied.to_string())?
            .project(id)
            .ok_or_else(|| "the project does not belong to the active profile".to_owned()),
        None => context
            .project()
            .map_err(|denied| denied.to_string())?
            .ok_or_else(|| "no project is active".to_owned()),
    }
}

/// Adds the project's identity to a result.
pub(crate) fn with_project(result: ActionResult, project: &Project) -> ActionResult {
    result
        .with_data("projectId", project.id.to_string())
        .with_data("projectKey", project.key.as_str())
        .with_data("projectName", project.name.clone())
}

/// Stable name of a resource kind, as serialized elsewhere.
pub fn kind_name(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Directory => "directory",
        ResourceKind::Application => "application",
        ResourceKind::Terminal => "terminal",
        ResourceKind::DockerEnvironment => "dockerEnvironment",
        ResourceKind::Service => "service",
        ResourceKind::Url => "url",
        ResourceKind::Other => "other",
    }
}

/// Resolves resource `role` of the target project. The result carries the
/// project, the resource name, its logical key, kind and the machine value.
pub(crate) fn resolve(
    parameters: &ActionParameters,
    context: &ExecutionContext<'_>,
    role: &str,
) -> ActionResult {
    if role.is_empty() {
        return ActionResult::failure("no resource configured");
    }
    let project = match target_project(parameters, context) {
        Ok(project) => project,
        Err(message) => return ActionResult::failure(message),
    };
    match context.resolve_project_resource(project, role) {
        Ok(binding) => with_project(
            ActionResult::success(format!("{} {role}: {}", project.key, binding.value)),
            project,
        )
        .with_data("resource", role)
        .with_data("resourceKey", binding.key.as_str())
        .with_data("kind", kind_name(binding.kind))
        .with_data("value", binding.value.clone()),
        Err(error) => with_project(ActionResult::failure(error.to_string()), project)
            .with_data("resource", role),
    }
}
