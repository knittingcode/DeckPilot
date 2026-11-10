//! The `projects` feature module.
//!
//! Projects belong to a profile. This module provides everything built on
//! them: the action types that switch and inspect the active project
//! ([`actions`]), and the operations that manage a profile's projects and
//! their order ([`catalog`]). The project data itself and the per-profile
//! active project context are part of the core profile model, so that
//! templates, resource resolution and workflow conditions work without this
//! module.

pub mod actions;
pub mod catalog;
pub mod resources;
pub mod switching;

use deckpilot_core::{
    ActionHandler, ActionParameters, Capability, FeatureModule, ModuleId, ModuleManifest,
    ModuleMigration, ModulePage, ParameterKind, ParameterSpec, ParameterValue, Portability,
    SettingScope,
};

pub use catalog::{CatalogError, ProjectCatalog};

pub const MODULE_ID: &str = "projects";
/// Module setting: whether next/previous wrap around at the ends.
pub const WRAP_AROUND: &str = "wrapAround";

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

#[derive(Debug)]
pub struct ProjectsModule {
    manifest: ModuleManifest,
}

impl ProjectsModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "Projects".into(),
                description: "Projects of a profile, their logical resources and actions that \
                              switch between them."
                    .into(),
                platforms: Vec::new(),
                pages: vec![ModulePage {
                    id: "projects".into(),
                    title: "Projects".into(),
                    icon: "folder-kanban".into(),
                    order: 10,
                }],
                capabilities: vec![
                    Capability::ProfileRead,
                    Capability::ProjectContext,
                    Capability::MachineResources,
                ],
                config: vec![ParameterSpec {
                    key: WRAP_AROUND.into(),
                    label: "Wrap around".into(),
                    description: Some(
                        "Next after the last project selects the first one, and vice versa.".into(),
                    ),
                    kind: ParameterKind::Boolean,
                    required: true,
                    default: Some(ParameterValue::Boolean(true)),
                }],
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: Portability {
                    version: 1,
                    settings: [(WRAP_AROUND.to_owned(), SettingScope::Portable)].into(),
                },
                // The projects tables were created by core schema 3, before
                // modules could own tables. Version 1 records that this
                // module owns them; later changes go here.
                migrations: vec![ModuleMigration {
                    version: 1,
                    description: "Adopt the projects, default_projects and active_projects tables"
                        .into(),
                    sql: String::new(),
                }],
            },
        }
    }
}

impl Default for ProjectsModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for ProjectsModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        let wrap = config.boolean(WRAP_AROUND).unwrap_or(true);
        actions::handlers(wrap)
    }
}
