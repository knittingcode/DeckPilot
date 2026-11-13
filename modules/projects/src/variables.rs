//! The variables the projects module owns: the project in the context.
//!
//! `projects.current.id`, `.key` and `.name` used to be the core system
//! variables `system.project.current.…`; those ids stay readable as
//! aliases (`deckpilot_core::variables::ALIASES`). Values come from the
//! explicit resolution context: inside a run its own project, for a key
//! press the active project. They are computed on every resolution.

use deckpilot_core::variables::{
    DeclaredScope, Requirement, Resolution, ValueSource, VariableDeclaration, VariableEnvironment,
    VariableSource, VariableType, VariableValue,
};

pub const CURRENT_ID: &str = "projects.current.id";
pub const CURRENT_KEY: &str = "projects.current.key";
pub const CURRENT_NAME: &str = "projects.current.name";

/// The declarations of the module's variables.
pub fn declarations() -> Vec<VariableDeclaration> {
    let project = |id: &str, name: &str, value_type, description: &str| {
        VariableDeclaration::new(id, name, value_type, DeclaredScope::Project)
            .describe(description)
            .requires(Requirement::Project)
    };
    vec![
        project(
            CURRENT_ID,
            "Project id",
            VariableType::ProjectId,
            "Id of the project in the context",
        ),
        project(
            CURRENT_KEY,
            "Project key",
            VariableType::String,
            "Key of the project in the context",
        ),
        project(
            CURRENT_NAME,
            "Project name",
            VariableType::String,
            "Name of the project in the context",
        ),
    ]
}

/// Resolves the module's variables from the context's project.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectVariables;

impl VariableSource for ProjectVariables {
    fn resolve(
        &self,
        declaration: &VariableDeclaration,
        environment: &VariableEnvironment<'_>,
    ) -> Resolution {
        let project = match environment.project() {
            Ok(project) => project,
            Err(resolution) => return resolution,
        };
        let value = match declaration.id.as_str() {
            CURRENT_ID => VariableValue::ProjectId(project.id),
            CURRENT_KEY => VariableValue::String(project.key.to_string()),
            CURRENT_NAME => VariableValue::String(project.name.clone()),
            _ => {
                return Resolution::Missing(
                    deckpilot_core::variables::MissingReason::UnknownVariable,
                );
            }
        };
        Resolution::Value {
            value,
            source: ValueSource::System,
            sensitive: false,
        }
    }
}
