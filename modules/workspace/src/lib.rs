//! The `workspace` feature module: opens a project's working set — its
//! repository (or a file in it, at a line) in an editor, and web pages in
//! the browser — from resources bound on this computer.
//!
//! It only opens things: there is no shell, nothing is run in a terminal,
//! and nothing is changed. Paths stay inside their resource (the System
//! module's rules), web pages must be `http` or `https` (the System
//! module's URL rules, without `mailto`), and editors and browsers are
//! application resources, never names typed into a workflow.

pub mod actions;
pub mod editor;

use std::collections::BTreeMap;
use std::sync::Arc;

use deckpilot_core::launch::{Launcher, SystemLauncher};
use deckpilot_core::{
    ActionHandler, ActionParameters, Capability, FeatureModule, ModuleId, ModuleManifest,
    Portability,
};

pub const MODULE_ID: &str = "workspace";

/// Stable error codes of the module's actions, besides the System
/// module's `invalid-url`, `invalid-path` and `not-found`.
pub mod codes {
    /// A line or column that is out of range, or given without a file.
    pub const INVALID_POSITION: &str = "invalid-position";
    /// No URL was given.
    pub const NO_URLS: &str = "no-urls";
    /// More URLs than allowed.
    pub const TOO_MANY_URLS: &str = "too-many-urls";
    /// `workspace.open-project` could open nothing.
    pub const NOTHING_OPENED: &str = "nothing-opened";
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

#[derive(Debug)]
pub struct WorkspaceModule {
    manifest: ModuleManifest,
    launcher: Arc<dyn Launcher>,
}

impl WorkspaceModule {
    pub fn new() -> Self {
        Self::with_launcher(Arc::new(SystemLauncher))
    }

    /// The module with another launcher, e.g. one that records plans.
    pub fn with_launcher(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "Workspace".into(),
                description: "Opens a project's repository or files in an editor and its pages \
                              in the browser, from resources bound on this computer."
                    .into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: vec![
                    Capability::ProfileRead,
                    Capability::MachineResources,
                    Capability::OpenExternal,
                ],
                config: Vec::new(),
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::new(),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
                variables: Vec::new(),
            },
            launcher,
        }
    }
}

impl Default for WorkspaceModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for WorkspaceModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, _: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        actions::handlers(&self.launcher)
    }
}
