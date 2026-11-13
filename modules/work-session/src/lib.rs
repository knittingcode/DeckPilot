//! The `work-session` feature module: a manifest only. It contributes the
//! Work Session page of the configuration app and the switch that enables
//! or disables it (docs/gestures-and-work-session.md §10).
//!
//! The session itself belongs to the agent and is operated from its key
//! only (decision D11), so the module has **no action types**: nothing a
//! workflow or another key could use to start, verify or finish a session,
//! and no action instance is ever created for it. Its settings are the
//! per-profile `work_session_settings`, not module configuration.

use std::collections::BTreeMap;

use deckpilot_core::{
    ActionHandler, ActionParameters, FeatureModule, ModuleId, ModuleManifest, ModulePage,
    Portability,
};

pub const MODULE_ID: &str = "work-session";
/// The page the configuration app shows for the module.
pub const PAGE_ID: &str = "work-session";

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

#[derive(Debug)]
pub struct WorkSessionModule {
    manifest: ModuleManifest,
}

impl WorkSessionModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "Work Session".into(),
                description: "Times work sessions started, checked and finished from a key, \
                              with their history."
                    .into(),
                platforms: Vec::new(),
                pages: vec![ModulePage {
                    id: PAGE_ID.into(),
                    title: "Work Session".into(),
                    icon: "timer".into(),
                    order: 20,
                }],
                capabilities: Vec::new(),
                config: Vec::new(),
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::new(),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
            },
        }
    }
}

impl Default for WorkSessionModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for WorkSessionModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, _: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_core::{ModuleRegistry, Os};

    use super::*;

    #[test]
    fn a_page_and_nothing_to_run() {
        let module = WorkSessionModule::new();
        let manifest = module.manifest();
        assert_eq!(manifest.id, module_id());
        assert_eq!(manifest.pages.len(), 1);
        assert_eq!(manifest.pages[0].id, "work-session");
        assert_eq!(manifest.capabilities.len(), 0);
        assert_eq!(manifest.config.len(), 0);
        assert_eq!(manifest.migrations.len(), 0);
        assert!(manifest.enabled_by_default);
        assert!(
            module
                .action_handlers(&ActionParameters::default())
                .is_empty()
        );

        let mut registry = ModuleRegistry::new(Os::current());
        registry.register(Box::new(module)).unwrap();
        let info = &registry.modules()[0];
        assert!(info.enabled && info.supported);
        assert_eq!(info.action_types.len(), 0);
        assert_eq!(registry.pages()[0].1.title, "Work Session");
    }
}
